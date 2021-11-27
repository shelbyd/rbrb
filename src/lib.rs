#![allow(unstable_name_collisions)]

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
//! - [x] Multi-party sync
//! - [ ] Consistent disconnection
//! - [ ] Reconnect disconnected player
//!
//! ## Robustness
//!
//! - [ ] Determinism checks
//! - [ ] Checksum propagation
//! - [ ] Debugging failed checks
//! - [x] Fake a bad network
//! - [x] Confirmation state
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

// Internal TODO list
// - share state checksums (probably with seahash)
// - adjust local simulation when behind/ahead
// - handle corrupted packets

use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    ops::ControlFlow,
    time::Duration,
};

mod builder;
pub use builder::SessionBuilder;
mod exponential_keeping;
mod inputs;
use inputs::InputStorage;
pub use inputs::{PlayerInputs, SerializedInput};
mod request_handler;
use request_handler::ControlFlowExt;
pub use request_handler::{Confirmation, Request, RequestHandler};
mod socket;
pub use socket::{BadSocket, BasicUdpSocket, NonBlockingSocket};
mod stats;
pub use stats::{BandwidthRecordingSocket, NetworkStats};
mod time;
use time::Interval;
mod utils;
use utils::div_duration;

pub type SerializedState = Vec<u8>;
pub type SimulationInstant = Duration;
pub type PlayerId = u16;

pub struct Session {
    confirmed_states: BTreeMap<Frame, SerializedState>,
    inputs: InputStorage,

    step_size: Duration,
    local_id: PlayerId,
    player_addresses: HashMap<SocketAddr, PlayerId>,
    socket: Box<dyn NonBlockingSocket>,

    host_at: SimulationInstant,
    unconfirmed: Frame,
    remote_unconfirmed: HashMap<PlayerId, Frame>,

    send_interval: Interval,
    shared_clock: time::SharedClock,
}

impl Session {
    pub fn players(&self) -> impl Iterator<Item = (PlayerId, Player)> + '_ {
        let remote = self
            .player_addresses
            .iter()
            .map(|(s, id)| (*id, Player::Remote(*s)));
        [(self.local_id, Player::Local)].into_iter().chain(remote)
    }

    pub fn network_stats(&self) -> NetworkStats {
        NetworkStats {
            drift: self.shared_clock.drift(),
            elapsed: self.shared_clock.signed_elapsed().unwrap_or_default(),
            socket: self.socket.stats(),
        }
    }

    pub fn next_request<H: RequestHandler>(&mut self, handler: H) -> ControlFlow<(), H::Break> {
        match self.next_request_flow_inverted(handler) {
            ControlFlow::Break(Some(m)) => ControlFlow::Continue(m),
            ControlFlow::Break(None) => ControlFlow::Break(()),
            ControlFlow::Continue(()) => ControlFlow::Break(()),
        }
    }

    fn next_request_flow_inverted<H: RequestHandler>(
        &mut self,
        mut handler: H,
    ) -> ControlFlow<Option<H::Break>> {
        loop {
            self.process_incoming_messages();
            self.send_messages();
            self.capture_inputs(&mut handler)?;
            self.save_frame_zero(&mut handler).map_break(Some)?;
            self.advance_confirmed_horizon(&mut handler)?;

            if !self.step_towards_realtime(&mut handler)? {
                return ControlFlow::Continue(());
            }
        }
    }

    fn step_towards_realtime<H: RequestHandler>(
        &mut self,
        handler: &mut H,
    ) -> ControlFlow<Option<H::Break>, bool> {
        let frame = self.host_frame();
        let clock_frame = self.clock_frame()?;
        match (frame.into_frame().cmp(&clock_frame), frame) {
            (Ordering::Greater, _) => {
                unreachable!("advanced too far: {:?} > {:?}", frame, clock_frame);
            }
            (Ordering::Equal, _) => return ControlFlow::Continue(false),
            (Ordering::Less, FrameState::At(_)) => {
                self.try_advance(handler, self.step_size).map_break(Some)?;
                // TODO(shelbyd): Do partial advance?
            }
            (Ordering::Less, FrameState::After(f, _)) => {
                self.navigate_to(f, handler).map_break(Some)?;
                self.try_advance(handler, self.step_size).map_break(Some)?;
                // TODO(shelbyd): Do partial advance?
            }
        }
        ControlFlow::Continue(true)
    }

    fn save_frame_zero<H: RequestHandler>(&mut self, handler: &mut H) -> ControlFlow<H::Break> {
        if self.confirmed_states.len() == 0 {
            assert_eq!(self.host_frame(), FrameState::At(Frame(0)));

            let state = self.confirmed_states.entry(Frame(0)).or_default();
            handler.handle_request(Request::SaveTo(state))?;
        }
        ControlFlow::Continue(())
    }

    fn should_save(&self, frame: Frame) -> bool {
        exponential_keeping::kept_set(self.unconfirmed.0).contains(&frame.0)
            && !self.confirmed_states.contains_key(&frame)
    }

    fn clear_states(&mut self) {
        let kept = exponential_keeping::kept_set(self.unconfirmed.0);
        for key in self.confirmed_states.keys().cloned().collect::<Vec<_>>() {
            if !kept.contains(&key.0) {
                self.confirmed_states.remove(&key);
            }
        }
    }

    fn capture_inputs<H: RequestHandler>(
        &mut self,
        handler: &mut H,
    ) -> ControlFlow<Option<H::Break>> {
        let realtime = self.clock_frame()?;
        if let Some(vec) = self.inputs.capture_into(realtime, self.local_id) {
            handler
                .handle_request(Request::CaptureLocalInput(vec))
                .map_break(Some)?;
        }
        ControlFlow::Continue(())
    }

    fn advance_confirmed_horizon<H: RequestHandler>(
        &mut self,
        handler: &mut H,
    ) -> ControlFlow<Option<H::Break>> {
        loop {
            let last_confirmed = self.unconfirmed - 1;
            let host_frame = self.host_frame().into_frame();

            let should_advance = host_frame < self.clock_frame()?;
            if !should_advance {
                return ControlFlow::Continue(());
            }

            let behind = (host_frame.0 - last_confirmed.0) * self.step_size;
            if behind > Duration::from_secs(1) {
                log::warn!("confirmation horizon {:?} behind", behind);
            }

            match self.inputs(last_confirmed) {
                None => return ControlFlow::Continue(()),
                Some(inputs) if !inputs.is_fully_confirmed(self.player_addresses.len()) => {
                    return ControlFlow::Continue(())
                }
                Some(inputs) => {
                    let inputs = inputs.clone();
                    self.navigate_to(last_confirmed, handler).map_break(Some)?;

                    self.advance_with(inputs, handler, self.step_size, true)
                        .always(|| self.unconfirmed = self.unconfirmed + 1)
                        .map_break(Some)?;
                }
            }
        }
    }

    fn navigate_to<H: RequestHandler>(
        &mut self,
        frame: Frame,
        handler: &mut H,
    ) -> ControlFlow<H::Break> {
        loop {
            let current_frame = self.host_frame().into_frame();

            if self.should_save(current_frame) {
                self.clear_states();

                let state = self.confirmed_states.entry(current_frame).or_default();
                handler.handle_request(Request::SaveTo(state))?;
            }

            match current_frame.cmp(&frame) {
                Ordering::Equal => return ControlFlow::Continue(()),
                Ordering::Greater => {
                    let (roll_to, state) = self
                        .confirmed_states
                        .range(..=frame)
                        .next_back()
                        .expect("should have at least one confirmed state");

                    let delta = current_frame.0 - roll_to.0;
                    if delta * self.step_size > Duration::from_millis(300) {
                        log::info!("rolling back {} frames to {:?}", delta, roll_to);
                    }

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
                } else if inputs.is_fully_confirmed(self.player_addresses.len()) {
                    Confirmation::Subsequent
                } else {
                    Confirmation::Unconfirmed
                },
                inputs,
            })
            .always(|| self.host_at += amount)
    }

    fn do_advance<H: RequestHandler>(&mut self, handler: &mut H) -> ControlFlow<H::Break> {
        let frame = match self.host_frame() {
            FrameState::At(f) => f,
            FrameState::After(_, _) => unimplemented!("FrameState::After"),
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
        let frame = match self.host_frame() {
            FrameState::At(f) => f,
            FrameState::After(_, _) => unimplemented!("FrameState::After"),
        };

        if let Some(inputs) = self.inputs(frame) {
            self.advance_with(inputs, handler, amount, false)?;
        }
        ControlFlow::Continue(())
    }

    fn clock_frame<T>(&self) -> ControlFlow<Option<T>, Frame> {
        match self.shared_clock.elapsed() {
            Some(dur) => ControlFlow::Continue(self.calculate_frame_state(dur).into_frame()),
            None => ControlFlow::Break(None),
        }
    }

    fn host_frame(&self) -> FrameState {
        self.calculate_frame_state(self.host_at)
    }

    fn calculate_frame_state(&self, at: Duration) -> FrameState {
        let (n, rem) = div_duration(at, self.step_size);
        if rem == Duration::ZERO {
            FrameState::At(Frame(n))
        } else {
            FrameState::After(Frame(n), rem)
        }
    }

    fn inputs(&self, at: Frame) -> Option<PlayerInputs> {
        self.inputs.at_frame(at)
    }

    fn send_messages(&mut self) {
        while let Some((addr, message)) = self.shared_clock.message() {
            self.send_to_addr(&Message::Clock(message), addr);
        }
        if !self.send_interval.is_time() {
            return;
        }

        for (player, unc) in self.remote_unconfirmed.clone() {
            let inputs = self.inputs.player_since_frame(self.local_id, unc);
            self.send_to(&Message::Inputs(inputs), player);
        }

        self.send(Message::Unconfirmed(self.unconfirmed - 1));
    }

    fn send(&mut self, message: Message) {
        let message = bincode::serialize(&message).expect("failed to serialize message");
        for player in self.player_addresses.keys() {
            self.socket.send(&message, *player);
        }
    }

    fn send_to(&mut self, message: &Message, player: PlayerId) {
        let addr = *self
            .player_addresses
            .iter()
            .find(|(_, &id)| id == player)
            .unwrap()
            .0;
        self.send_to_addr(message, addr);
    }

    fn send_to_addr(&mut self, message: &Message, addr: SocketAddr) {
        let message = bincode::serialize(&message).expect("failed to serialize message");
        self.socket.send(&message, addr);
    }

    fn process_incoming_messages(&mut self) {
        while let Some((addr, buffer)) = self.socket.recv() {
            let player = match self.player_addresses.get(&addr) {
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
                Message::Inputs(map) => {
                    self.inputs.merge_remote(*player, map);
                }
                Message::Unconfirmed(frame) => {
                    let unc = self.remote_unconfirmed.entry(*player).or_insert(frame);
                    *unc = std::cmp::max(*unc, frame);
                }
                Message::Clock(m) => {
                    self.shared_clock.receive_message(addr, m);
                }
            }
        }
    }
}

pub enum Player {
    Local,
    Remote(SocketAddr),
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
    After(Frame, Duration),
}

impl FrameState {
    fn into_frame(self) -> Frame {
        match self {
            FrameState::At(f) => f,
            FrameState::After(f, _) => f,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum Message {
    Inputs(BTreeMap<Frame, Vec<u8>>),
    Unconfirmed(Frame),
    Clock(time::ClockMessage),
}
