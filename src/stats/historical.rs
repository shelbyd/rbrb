use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub struct Historical {
    map: BTreeMap<Instant, u64>,
    keep_for: Duration,
}

impl Historical {
    pub fn over_secs(secs: u64) -> Self {
        Historical {
            map: Default::default(),
            keep_for: Duration::from_secs(secs),
        }
    }

    pub fn clean(&mut self) {
        match self.map.range(..).next() {
            None => return,
            Some((at, _)) if at.elapsed() < self.keep_for * 2 => return,
            _ => {
                self.map = self.map.split_off(&(Instant::now() - self.keep_for));
            }
        }
    }

    pub fn increment(&mut self, amount: u64) {
        self.map.insert(Instant::now(), amount);
    }

    pub fn avg_per_sec(&self) -> u64 {
        let include_after = Instant::now() - self.keep_for;
        self.map
            .range(include_after..)
            .map(|(_, amt)| amt)
            .sum::<u64>()
            / self.keep_for.as_secs()
    }
}
