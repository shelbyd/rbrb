use serde::*;
use std::{
    collections::HashSet,
    net::SocketAddr,
    time::{Duration, Instant},
};

pub struct Interval {
    last: Option<Instant>,
    every: Duration,
}

impl Interval {
    pub fn new(every: Duration) -> Self {
        Interval { last: None, every }
    }

    pub fn is_time(&mut self) -> bool {
        match self.last.as_mut() {
            Some(at) if at.elapsed() < self.every => return false,
            Some(at) => *at += self.every,
            None => self.last = Some(Instant::now()),
        }
        true
    }
}

#[derive(Debug)]
pub struct SharedClock {
    state: ClockState,
    remotes: HashSet<SocketAddr>,
}

impl SharedClock {
    pub fn among_remotes(remotes: impl IntoIterator<Item = SocketAddr>) -> Self {
        SharedClock {
            state: ClockState::Started(Instant::now()),
            remotes: remotes.into_iter().collect(),
        }
    }

    pub fn message(&mut self) -> Option<(SocketAddr, ClockMessage)> {
        None
    }

    pub fn receive_message(&mut self, _from: SocketAddr, message: ClockMessage) {
        match message {
            unhandled => unimplemented!("{:?}", unhandled),
        }
    }

    pub fn elapsed(&self) -> Option<Duration> {
        match self.state {
            ClockState::Synchronizing => None,
            ClockState::Started(at) => Some(at.elapsed()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ClockState {
    #[allow(dead_code)]
    Synchronizing,
    Started(Instant),
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ClockMessage {
    StartIn(Duration),
}
