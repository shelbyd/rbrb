//! A library for building RoBust RollBack-based networked games.
//!
//! `rbrb` is heavily inspired by [GGPO](https://www.ggpo.net/) and
//! [GGRS](https://github.com/gschup/ggrs), but aims to be more reliable and capable.
//!
//! # Assumptions
//!
//! This library assumes your game is a deterministic `Fn(&State, Set<Input>) -> State`.
//! We (will) have an additional testing mode that will spend extra cycles on checking that the
//! state is consitent between players and deterministic on the same logical update.
//!
//! # Roadmap
//!
//! ## Core Functionality
//!
//! - [ ] Multi-party sync
//! - [ ] Consistent disconnection
//! - [ ] Reconnect disconnected player
//!
//! ## Robustness
//!
//! - [ ] Determinism checks
//! - [ ] Checksum propagation
//! - [ ] Debugging failed checks
//! - [ ] Fake a bad network
//! - [ ] Confirmation state
//!
//! ## Features
//!
//! - [ ] In-game replays
//! - [ ] Out of game replays
//!   - [ ] Headless
//! - [ ] Spectators
//!   - [ ] Drop in/out
//!
//! ## Performance
//!
//! - [ ] Input delta encoding
//! - [ ] Hub and spoke network

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    ops::ControlFlow,
    time::{Duration, Instant},
};

mod socket;
pub use socket::NonBlockingSocket;

pub type SerializedState = Vec<u8>;
pub type SerializedInput = Vec<u8>;
pub type SimulationInstant = Duration;
pub type PlayerId = u16;

pub struct Session {
    saved_states: BTreeMap<Frame, SerializedState>,
    saved_inputs: BTreeMap<Frame, PlayerInputs>,

    step_size: Duration,
    local_index: PlayerId,
    remote_players: HashMap<SocketAddr, PlayerId>,

    started_at: Instant,
    host_at: Option<SimulationInstant>,
    socket: Box<dyn NonBlockingSocket>,
}

impl Session {
    pub fn next_request<R>(
        &mut self,
        handler: impl FnOnce(Request) -> R,
    ) -> ControlFlow<Option<R>, R> {
        self.process_incoming_messages();

        match self.frame_state() {
            None => {
                let mut state = Vec::new();
                let ret = handler(Request::SaveTo(&mut state));
                self.host_at = Some(Duration::ZERO);
                self.saved_states.insert(Frame(0), state);
                return ControlFlow::Continue(ret);
            }
            Some(FrameState::At(frame)) => {
                let inputs = match self.inputs(frame) {
                    Some(i) => i,
                    None => {
                        let mut input = Vec::new();
                        let ret = handler(Request::CaptureLocalInput(&mut input));

                        assert!(
                            !self.saved_inputs.contains_key(&frame),
                            "overrode inputs for frame {:?}",
                            frame
                        );

                        self.send(Message::Input(frame, &input[..]));
                        self.saved_inputs
                            .insert(frame, PlayerInputs::just_local(self.local_index, input));

                        return ControlFlow::Continue(ret);
                    }
                };

                let next_frame = frame + 1;
                if self.started_at.elapsed() > (self.step_size * next_frame.0) {
                    let ret = handler(Request::Advance(self.step_size, inputs));
                    *self.host_at.as_mut().unwrap() += self.step_size;
                    return ControlFlow::Continue(ret);
                }
            }
            Some(FrameState::After(_)) => unimplemented!("FrameState::After"),
        }

        ControlFlow::Break(None)
    }

    fn frame_state(&self) -> Option<FrameState> {
        // TODO(shelbyd): Extract into algorithms crate.
        use core::cmp::Ordering;

        let at = self.host_at?;
        let step = self.step_size;

        let mut min = 0;
        let mut max = 1;

        loop {
            if step * min == at {
                return Some(FrameState::At(Frame(min)));
            }
            if step * max > at {
                break;
            }
            min = max;
            max *= 2;
        }

        while max - min > 1 {
            let mid = min + (max - min) / 2;
            match (step * mid).cmp(&at) {
                Ordering::Equal => return Some(FrameState::At(Frame(mid))),
                Ordering::Greater => {
                    max = mid;
                }
                Ordering::Less => {
                    min = mid;
                }
            }
        }
        Some(FrameState::After(Frame(min)))
    }

    fn inputs(&self, at: Frame) -> Option<PlayerInputs> {
        self.saved_inputs.get(&at).cloned()
    }

    fn send(&mut self, message: Message) {
        let message = bincode::serialize(&message).expect("failed to serialize message");
        for player in self.remote_players.keys() {
            self.socket.send(&message, *player);
        }
    }

    fn process_incoming_messages(&mut self) {
        while let Some((addr, buffer)) = self.socket.recv() {
            let player = match self.remote_players.get(&addr) {
                Some(p) => p,
                None => {
                    log::warn!("got message from non-player: {}", addr);
                    continue;
                }
            };
            let message = match bincode::deserialize(buffer) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("failed to decode message: {:?}", e);
                    continue;
                }
            };
            match message {
                Message::Input(frame, input) => {
                    self.saved_inputs
                        .get_mut(&frame)
                        .unwrap()
                        .insert(*player, input.to_vec());
                }
            }
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Request<'s> {
    SaveTo(&'s mut SerializedState),
    Advance(Duration, PlayerInputs),
    CaptureLocalInput(&'s mut SerializedInput),
}

#[derive(Debug, Clone)]
pub struct PlayerInputs<T = SerializedInput> {
    map: HashMap<PlayerId, T>,
}

impl PlayerInputs {
    fn just_local(local_index: PlayerId, input: SerializedInput) -> Self {
        let mut map = HashMap::default();
        map.insert(local_index, input);
        PlayerInputs { map }
    }
}

impl<T> PlayerInputs<T> {
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> PlayerInputs<U> {
        PlayerInputs {
            map: self.map.into_iter().map(|(k, v)| (k, f(v))).collect(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PlayerId, &T)> {
        self.map.iter()
    }

    fn insert(&mut self, at: PlayerId, val: T) {
        let already = self.map.insert(at, val);
        assert!(already.is_none(), "inserted duplicate input for player {}", at);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
struct Frame(u32);

impl core::ops::Add<u32> for Frame {
    type Output = Frame;
    fn add(self, other: u32) -> Frame {
        Frame(self.0 + other)
    }
}

#[derive(Clone, Copy, Debug)]
enum FrameState {
    At(Frame),
    After(Frame),
}

#[derive(Serialize, Deserialize)]
enum Message<'i> {
    Input(Frame, &'i [u8]),
}

#[derive(Default)]
pub struct SessionBuilder {
    remote_players: Vec<SocketAddr>,
    local_player: Option<(PlayerId, u16)>,
    step_size: Option<Duration>,
}

impl SessionBuilder {
    pub fn remote_players(&mut self, players: &[SocketAddr]) -> &mut Self {
        self.remote_players = players.to_vec();
        self
    }

    pub fn local_player(&mut self, index: PlayerId, port: u16) -> &mut Self {
        self.local_player = Some((index, port));
        self
    }

    pub fn step_size(&mut self, size: Duration) -> &mut Self {
        self.step_size = Some(size);
        self
    }

    pub fn start(&mut self) -> Result<Session, String> {
        let (local_index, port) = self.local_player.ok_or("must provide local_player")?;

        let remote_players = self
            .remote_players
            .iter()
            .enumerate()
            .map(|(i, &addr)| {
                let i = i as u16;
                if i >= local_index {
                    (addr, i + 1)
                } else {
                    (addr, i)
                }
            })
            .collect();

        Ok(Session {
            saved_states: BTreeMap::default(),
            saved_inputs: BTreeMap::default(),
            host_at: None,
            started_at: Instant::now(),
            step_size: self.step_size.ok_or("must provide step_size")?,
            local_index,
            socket: Box::new(socket::BasicUdpSocket::bind(port).unwrap()),
            remote_players,
        })
    }
}
