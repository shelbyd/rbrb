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

use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    ops::ControlFlow,
    time::{Duration, Instant},
};

pub type SerializedState = Vec<u8>;
pub type SerializedInput = Vec<u8>;
pub type SimulationInstant = Duration;
pub type PlayerId = u16;

pub struct Session {
    saved_states: BTreeMap<SimulationInstant, SerializedState>,
    saved_inputs: BTreeMap<SimulationInstant, PlayerInputs>,

    step_size: Duration,
    local_index: PlayerId,

    started_at: Instant,
    host_at: Option<SimulationInstant>,
}

impl Session {
    pub fn next_request<R>(
        &mut self,
        handler: impl FnOnce(Request) -> R,
    ) -> ControlFlow<Option<R>, R> {
        let host_at = match self.host_at {
            Some(h) => h,
            None => {
                let mut state = Vec::new();
                let ret = handler(Request::SaveTo(&mut state));
                self.host_at = Some(Duration::ZERO);
                self.saved_states.insert(Duration::ZERO, state);
                return ControlFlow::Continue(ret);
            }
        };

        if self.started_at.elapsed() > (host_at + self.step_size) {
            let inputs = match self.inputs(host_at) {
                Some(i) => i,
                None => {
                    let mut input = Vec::new();
                    let ret = handler(Request::CaptureLocalInput(&mut input));
                    self.saved_inputs
                        .insert(host_at, PlayerInputs::just_local(self.local_index, input));
                    return ControlFlow::Continue(ret);
                }
            };

            let ret = handler(Request::Advance(self.step_size, inputs));
            *self.host_at.as_mut().unwrap() += self.step_size;
            return ControlFlow::Continue(ret);
        }

        ControlFlow::Break(None)
    }

    fn inputs(&self, at: SimulationInstant) -> Option<PlayerInputs> {
        self.saved_inputs.get(&at).cloned()
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
        let (local_index, _port) = self.local_player.ok_or("must provide local_player")?;

        Ok(Session {
            saved_states: BTreeMap::default(),
            saved_inputs: BTreeMap::default(),
            host_at: None,
            started_at: Instant::now(),
            step_size: self.step_size.ok_or("must provide step_size")?,
            local_index,
        })
    }
}
