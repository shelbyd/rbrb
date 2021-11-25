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
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    ops::ControlFlow,
    time::{Duration, Instant},
};

mod builder;
pub use builder::SessionBuilder;
mod exponential_keeping;
mod request_handler;
use request_handler::ControlFlowExt;
pub use request_handler::{Confirmation, Request, RequestHandler};
mod socket;
pub use socket::{BadSocket, NonBlockingSocket};

pub type SerializedState = Vec<u8>;
pub type SerializedInput = Vec<u8>;
pub type SimulationInstant = Duration;
pub type PlayerId = u16;

pub struct Session {
    confirmed_states: BTreeMap<Frame, SerializedState>,
    saved_inputs: BTreeMap<Frame, PlayerInputs>,

    step_size: Duration,
    local_id: PlayerId,
    remote_players: HashMap<SocketAddr, PlayerId>,
    socket: Box<dyn NonBlockingSocket>,

    started_at: Instant,
    host_at: SimulationInstant,
    unconfirmed: Frame,
}

impl Session {
    pub fn players(&self) -> impl Iterator<Item = (PlayerId, Player)> + '_ {
        let remote = self
            .remote_players
            .iter()
            .map(|(s, id)| (*id, Player::Remote(*s)));
        [(self.local_id, Player::Local)]
            .into_iter()
            .chain(remote)
    }

    pub fn next_request<H: RequestHandler>(&mut self, handler: H) -> ControlFlow<(), H::Break> {
        self.process_incoming_messages();

        match self.next_request_flow_inverted(handler) {
            ControlFlow::Break(m) => ControlFlow::Continue(m),
            ControlFlow::Continue(()) => ControlFlow::Break(()),
        }
    }

    fn next_request_flow_inverted<H: RequestHandler>(
        &mut self,
        mut handler: H,
    ) -> ControlFlow<H::Break> {
        loop {
            // TODO(shelbyd): Wait for all players to connect to start the simulation.
            self.capture_inputs(&mut handler)?;
            self.save_confirmed_frames(&mut handler)?;
            self.advance_confirmed_horizon(&mut handler)?;

            let frame = self.frame_state();
            match (frame.into_frame().cmp(&self.realtime_frame()), frame) {
                (Ordering::Greater, f) => {
                    unreachable!("advanced too far: {:?} > {:?}", f, self.realtime_frame());
                }
                (Ordering::Equal, _) => return ControlFlow::Continue(()),
                (Ordering::Less, FrameState::At(_)) => {
                    self.try_advance(&mut handler, self.step_size)?;
                    // TODO(shelbyd): Do partial advance?
                }
                (Ordering::Less, FrameState::After(f)) => {
                    self.navigate_to(f, &mut handler)?;
                    self.try_advance(&mut handler, self.step_size)?;
                    // TODO(shelbyd): Do partial advance?
                }
            }
        }
    }

    fn save_confirmed_frames<H: RequestHandler>(
        &mut self,
        handler: &mut H,
    ) -> ControlFlow<H::Break> {
        if self.confirmed_states.len() == 0 {
            let mut state = Vec::new();
            handler
                .handle_request(Request::SaveTo(&mut state))
                .always(|| {
                    self.confirmed_states.insert(Frame(0), state);
                })?;
        }

        let current_frame = self.frame_state().into_frame();
        let kept = exponential_keeping::kept_set((self.unconfirmed - 1).0);
        if kept.contains(&current_frame.0) {
            if let None = self.confirmed_states.get(&current_frame) {
                let mut state = Vec::new();
                handler
                    .handle_request(Request::SaveTo(&mut state))
                    .always(|| {
                        self.confirmed_states.insert(current_frame, state);

                        for key in self.confirmed_states.keys().cloned().collect::<Vec<_>>() {
                            if !kept.contains(&key.0) {
                                self.confirmed_states.remove(&key);
                            }
                        }
                    })?;
            }
        }

        ControlFlow::Continue(())
    }

    fn capture_inputs<H: RequestHandler>(&mut self, handler: &mut H) -> ControlFlow<H::Break> {
        let realtime = self.realtime_frame();
        if self.saved_inputs.contains_key(&realtime) {
            return ControlFlow::Continue(());
        }

        let mut input = Vec::new();
        handler
            .handle_request(Request::CaptureLocalInput(&mut input))
            .always(|| loop {
                let insert_into_frame = self
                    .saved_inputs
                    .range(..)
                    .next_back()
                    .map(|(f, _)| *f + 1)
                    .unwrap_or(Frame(0));
                self.send(Message::Input(insert_into_frame, &input[..]));
                let inputs = PlayerInputs::just_local(self.local_id, input.clone());
                self.saved_inputs.insert(insert_into_frame, inputs);
                if insert_into_frame == realtime {
                    break;
                } else {
                    log::warn!("missed capturing input for frame {:?}", insert_into_frame);
                }
            })
    }

    fn advance_confirmed_horizon<H: RequestHandler>(
        &mut self,
        handler: &mut H,
    ) -> ControlFlow<H::Break> {
        let last_confirmed = self.unconfirmed - 1;
        let current_frame = self.frame_state().into_frame();
        let should_advance = current_frame < self.realtime_frame();
        if !should_advance {
            return ControlFlow::Continue(());
        }

        match self.saved_inputs.get(&last_confirmed) {
            None => {}
            Some(inputs) if !inputs.is_complete(self.remote_players.len()) => {}
            Some(inputs) => {
                let inputs = inputs.clone();
                self.navigate_to(last_confirmed, handler)?;

                self.advance_with(inputs, handler, self.step_size, true)
                    .always(|| self.unconfirmed = self.unconfirmed + 1)?;
            }
        }

        ControlFlow::Continue(())
    }

    fn navigate_to<H: RequestHandler>(
        &mut self,
        frame: Frame,
        handler: &mut H,
    ) -> ControlFlow<H::Break> {
        loop {
            let current_frame = self.frame_state().into_frame();
            match current_frame.cmp(&frame) {
                Ordering::Equal => return ControlFlow::Continue(()),
                Ordering::Greater => {
                    let (roll_to, state) =
                        self.confirmed_states.range(..=frame).next_back().unwrap();
                    log::debug!("rolling back {} frames", current_frame.0 - roll_to.0);
                    handler
                        .handle_request(Request::LoadFrom(&state))
                        .always(|| {
                            self.host_at = self.step_size * roll_to.0;
                        })?;
                }
                Ordering::Less => {
                    self.do_advance(handler)?;
                }
            }
        }
    }

    fn advance_with<H: RequestHandler>(
        &mut self,
        inputs: PlayerInputs,
        handler: &mut H,
        amount: Duration,
        first_confirm: bool,
    ) -> ControlFlow<H::Break> {
        handler
            .handle_request(Request::Advance {
                amount,
                confirmed: if first_confirm {
                    Confirmation::First
                } else if inputs.is_complete(self.remote_players.len()) {
                    Confirmation::Subsequent
                } else {
                    Confirmation::Unconfirmed
                },
                inputs,
            })
            .always(|| self.host_at += amount)
    }

    fn do_advance<H: RequestHandler>(&mut self, handler: &mut H) -> ControlFlow<H::Break> {
        let frame = match self.frame_state() {
            FrameState::At(f) => f,
            FrameState::After(_) => unimplemented!("FrameState::After"),
        };
        let inputs = self
            .inputs(frame)
            .expect(&format!("did not have inputs for frame: {:?}", frame));

        self.advance_with(inputs, handler, self.step_size, false)
    }

    fn try_advance<H: RequestHandler>(
        &mut self,
        handler: &mut H,
        amount: Duration,
    ) -> ControlFlow<H::Break> {
        let frame = match self.frame_state() {
            FrameState::At(f) => f,
            FrameState::After(_) => unimplemented!("FrameState::After"),
        };

        if let Some(inputs) = self.inputs(frame) {
            self.advance_with(inputs, handler, amount, false)?;
        }
        ControlFlow::Continue(())
    }

    fn realtime_frame(&self) -> Frame {
        self.calculate_frame_state(self.started_at.elapsed())
            .into_frame()
    }

    fn frame_state(&self) -> FrameState {
        self.calculate_frame_state(self.host_at)
    }

    fn calculate_frame_state(&self, at: Duration) -> FrameState {
        // TODO(shelbyd): Extract into algorithms crate.

        let step = self.step_size;

        let mut min = 0;
        let mut max = 1;

        loop {
            if step * min == at {
                return FrameState::At(Frame(min));
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
                Ordering::Equal => return FrameState::At(Frame(mid)),
                Ordering::Greater => {
                    max = mid;
                }
                Ordering::Less => {
                    min = mid;
                }
            }
        }
        FrameState::After(Frame(min))
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
            log::debug!("(player, message): {:?}", (player, &message));
            match message {
                Message::Input(frame, input) => {
                    self.saved_inputs
                        .entry(frame)
                        .or_default()
                        .insert(*player, input.to_vec());
                }
            }
        }
    }
}

pub enum Player {
    Local,
    Remote(SocketAddr),
}

#[derive(Debug, Clone, Default)]
pub struct PlayerInputs<T = SerializedInput> {
    map: HashMap<PlayerId, T>,
}

impl PlayerInputs {
    fn just_local(local_id: PlayerId, input: SerializedInput) -> Self {
        let mut map = HashMap::default();
        map.insert(local_id, input);
        PlayerInputs { map }
    }

    fn is_complete(&self, remote_count: usize) -> bool {
        let should_have = remote_count + 1;
        let len = self.map.len();
        assert!(len <= should_have);
        len == should_have
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

impl core::ops::Sub<u32> for Frame {
    type Output = Frame;
    fn sub(self, other: u32) -> Frame {
        Frame(self.0 - other)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameState {
    At(Frame),
    After(Frame),
}

impl FrameState {
    fn into_frame(self) -> Frame {
        match self {
            FrameState::At(f) => f,
            FrameState::After(f) => f,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum Message<'i> {
    Input(Frame, &'i [u8]),
}
