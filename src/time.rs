use std::time::{Duration, Instant};
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
            Some(at) if at.elapsed() < self.every => false,
            Some(at) => {
                *at += self.every;
                true
            }
            None => {
                self.last = Some(Instant::now());
                true
            }
        }
    }
}
