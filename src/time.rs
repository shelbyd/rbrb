use serde::*;
use std::{
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

pub struct SharedClock {
    started_at: Instant,
}

impl Default for SharedClock {
    fn default() -> Self {
        SharedClock {
            started_at: Instant::now(),
        }
    }
}

impl SharedClock {
    pub fn message(&mut self) -> Option<(SocketAddr, ClockMessage)> {
        None
    }

    pub fn receive_message(&mut self, _from: SocketAddr, _message: ClockMessage) {}

    pub fn elapsed(&self) -> Option<Duration> {
        Some(self.started_at.elapsed())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ClockMessage {}
