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
    pub fn next_request<H: RequestHandler>(&mut self, handler: H) -> ControlFlow<(), H::Break> {
        match self.next_request_flow_inverted(handler) {
            ControlFlow::Break(m) => ControlFlow::Continue(m),
            ControlFlow::Continue(()) => ControlFlow::Break(()),
        }
    }

    fn next_request_flow_inverted<H: RequestHandler>(
        &mut self,
        mut handler: H,
    ) -> ControlFlow<H::Break> {
        self.process_incoming_messages();

        let frame = self.current_frame(&mut handler)?;

        match frame {
            FrameState::At(frame) => {
                let inputs = self.inputs(frame, &mut handler)?;

                let next_frame = frame + 1;
                if self.started_at.elapsed() > (self.step_size * next_frame.0) {
                    handler
                        .handle_request(Request::Advance(self.step_size, inputs))
                        .always(|| {
                            *self.host_at.as_mut().unwrap() += self.step_size;
                        })?;
                }
            }
            FrameState::After(_) => unimplemented!("FrameState::After"),
        }

        ControlFlow::Continue(())
    }

    fn current_frame<H: RequestHandler>(
        &mut self,
        handler: &mut H,
    ) -> ControlFlow<H::Break, FrameState> {
        if let Some(f) = self.frame_state() {
            return ControlFlow::Continue(f);
        }

        let mut state = Vec::new();
        handler
            .handle_request(Request::SaveTo(&mut state))
            .always(|| {
                self.host_at = Some(Duration::ZERO);
                self.saved_states.insert(Frame(0), state);
                FrameState::At(Frame(0))
            })
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

    fn inputs<H: RequestHandler>(
        &mut self,
        at: Frame,
        handler: &mut H,
    ) -> ControlFlow<H::Break, PlayerInputs> {
        if let Some(i) = self.saved_inputs.get(&at).cloned() {
            return ControlFlow::Continue(i);
        }

        let mut input = Vec::new();
        handler
            .handle_request(Request::CaptureLocalInput(&mut input))
            .always(|| {
                self.send(Message::Input(at, &input[..]));
                let inputs = PlayerInputs::just_local(self.local_index, input);
                self.saved_inputs.insert(at, inputs.clone());
                inputs
            })
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

pub trait RequestHandler {
    type Break;

    fn handle_request(&mut self, request: Request) -> ControlFlow<Self::Break>;
}

impl<F, M> RequestHandler for F
where
    F: FnMut(Request) -> M,
    M: MaybeMessage,
{
    type Break = M::Message;

    fn handle_request(&mut self, request: Request) -> ControlFlow<Self::Break> {
        if let Some(m) = self(request).as_message() {
            ControlFlow::Break(m)
        } else {
            ControlFlow::Continue(())
        }
    }
}

pub trait MaybeMessage {
    type Message;

    fn as_message(self) -> Option<Self::Message>;
}

impl<M> MaybeMessage for ControlFlow<M> {
    type Message = M;

    fn as_message(self) -> Option<Self::Message> {
        match self {
            ControlFlow::Break(m) => Some(m),
            ControlFlow::Continue(()) => None,
        }
    }
}

impl<M> MaybeMessage for Option<M> {
    type Message = M;

    fn as_message(self) -> Option<Self::Message> {
        self
    }
}

impl MaybeMessage for () {
    // TODO(shelbyd): Should be never (!).
    type Message = ();

    fn as_message(self) -> Option<Self::Message> {
        None
    }
}

trait ControlFlowExt {
    type Break;
    fn always<R>(self, f: impl FnOnce() -> R) -> ControlFlow<Self::Break, R>;
}

impl<B> ControlFlowExt for ControlFlow<B> {
    type Break = B;

    fn always<R>(self, f: impl FnOnce() -> R) -> ControlFlow<B, R> {
        let ret = f();
        self?;
        ControlFlow::Continue(ret)
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
        assert!(
            already.is_none(),
            "inserted duplicate input for player {}",
            at
        );
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
