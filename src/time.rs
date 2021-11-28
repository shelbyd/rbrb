use rand::Rng;
use serde::*;
use std::{
    sync::RwLock,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    net::SocketAddr,
    time::{Duration, Instant},
};

use crate::utils::Signed;

#[derive(Debug)]
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

    pub fn set_every(&mut self, every: Duration) {
        self.every = every;
    }
}

#[derive(Debug)]
pub struct SharedClock {
    state: ClockState,
    remotes: HashMap<SocketAddr, NetworkQuality>,
    queue: VecDeque<(SocketAddr, ClockMessage)>,

    remote_elapsed: HashMap<SocketAddr, (Signed<Duration>, Instant)>,
    last_elapsed: RwLock<Duration>,
    drift: Signed<Duration>,
    adjust_drift: Interval,
}

impl SharedClock {
    pub fn among_remotes(remotes: impl IntoIterator<Item = SocketAddr>) -> Self {
        SharedClock {
            state: ClockState::Synchronizing,
            remotes: remotes
                .into_iter()
                .map(|addr| (addr, Default::default()))
                .collect(),
            queue: Default::default(),

            remote_elapsed: Default::default(),
            last_elapsed: RwLock::new(Duration::ZERO),
            drift: Signed::Pos(Duration::ZERO),
            adjust_drift: Interval::new(Duration::from_millis(100)),
        }
    }

    pub fn message(&mut self) -> Option<(SocketAddr, ClockMessage)> {
        None.or_else(|| self.queue.pop_front())
            .or_else(|| self.start_message())
            .or_else(|| {
                self.remotes
                    .iter_mut()
                    .filter_map(|(&addr, net)| {
                        Some((addr, ClockMessage::NetworkAnalysis(net.message()?)))
                    })
                    .next()
            })
    }

    fn start_message(&mut self) -> Option<(SocketAddr, ClockMessage)> {
        if let ClockState::Synchronizing = self.state {
            let worst_rtt = self
                .remotes
                .values()
                .map(|network| network.worst_case_rtt())
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .max()
                .unwrap_or_default();

            let confident_start_in = 10 * worst_rtt;
            self.update_start_time(Instant::now() + confident_start_in);
        }

        let message = ClockMessage::Elapsed(self.signed_elapsed()?);
        match &mut self.state {
            ClockState::Start {
                unacked,
                sync_start,
                ..
            } => {
                if !sync_start.is_time() {
                    return None;
                }

                if unacked.len() == 0 {
                    sync_start.set_every(Duration::from_millis(500));
                    self.queue
                        .extend(self.remotes.keys().map(|addr| (*addr, message.clone())));
                } else {
                    sync_start.set_every(Duration::from_millis(50));
                    self.queue
                        .extend(unacked.iter().map(|addr| (*addr, message.clone())));
                }
                self.queue.pop_front()
            }
            ClockState::Synchronizing => None,
        }
    }

    pub fn receive_message(&mut self, from: SocketAddr, message: ClockMessage) {
        match message {
            ClockMessage::NetworkAnalysis(m) => {
                self.remotes.get_mut(&from).unwrap().receive_message(m);
            }

            ClockMessage::Elapsed(amt) => {
                self.record_remote_elapsed(from, amt);
                self.adjust_drift();

                if let Some(rtt) = self.remotes[&from].average_rtt() {
                    let true_elapsed = amt - (rtt / 2).into();
                    let start_at = true_elapsed.sub_from(Instant::now());

                    if self.update_start_time(start_at) {
                        log::info!("now starting in {:?}", self.signed_elapsed().unwrap());
                    } else {
                        match &mut self.state {
                            ClockState::Start { unacked, .. } => {
                                unacked.remove(&from);
                                if rand::thread_rng().gen() {
                                    let message =
                                        ClockMessage::Elapsed(self.signed_elapsed().unwrap());
                                    self.queue.push_back((from, message));
                                }
                            }
                            ClockState::Synchronizing => unreachable!(),
                        }
                    }
                }
            }
        }
    }

    fn record_remote_elapsed(&mut self, from: SocketAddr, elapsed: Signed<Duration>) {
        let existing = self
            .remote_elapsed
            .entry(from)
            .or_insert_with(|| (elapsed, Instant::now()));
        if elapsed <= existing.0 {
            return;
        }

        existing.0 = elapsed;
        existing.1 = Instant::now();
    }

    fn adjust_drift(&mut self) {
        if !self.adjust_drift.is_time() {
            return;
        }
        let local_elapsed = match self.signed_elapsed() {
            Some(e) => e,
            None => return,
        };
        let avg_delta = self
            .remote_elapsed
            .iter()
            .filter_map(|(addr, &(elapsed, at))| {
                let remote_elapsed =
                    elapsed + at.elapsed().into() + (self.remotes[addr].average_rtt()? / 2).into();
                let delta = local_elapsed - remote_elapsed;
                Some(delta)
            })
            .sum::<Signed<Duration>>()
            / (self.remote_elapsed.len() as u32);

        let weighted_adjust = self.drift.map(|_| Duration::from_micros(100));
        let delta = -avg_delta + weighted_adjust;

        let max_change = Duration::from_millis(1);
        let change = delta.clamp(Signed::Neg(max_change), Signed::Pos(max_change));
        self.drift = self.drift + change;
    }

    fn update_start_time(&mut self, new_at: Instant) -> bool {
        match &self.state {
            ClockState::Synchronizing => {}
            ClockState::Start { at, .. } => {
                if *at < Instant::now() {
                    return false;
                }
                if duration_since(*at, new_at).abs() < Duration::from_millis(400) {
                    return false;
                }
                if *at > new_at {
                    return false;
                }
            }
        }

        log::info!(
            "connected, starting in {:?}",
            duration_since(new_at, Instant::now()),
        );
        self.state = ClockState::Start {
            at: new_at,
            unacked: self.remotes.keys().cloned().collect(),
            sync_start: Interval::new(Duration::from_millis(50)),
        };
        true
    }

    pub fn elapsed(&self) -> Option<Duration> {
        let correct = self.signed_elapsed()?.pos()?;
        let mut lock = self.last_elapsed.write().unwrap();
        let never_decrease = std::cmp::max(correct, *lock);
        *lock = never_decrease;
        Some(never_decrease)
    }

    pub fn signed_elapsed(&self) -> Option<Signed<Duration>> {
        match self.state {
            ClockState::Synchronizing => None,
            ClockState::Start { at, .. } => {
                let only_local = duration_since(Instant::now(), at);
                Some(only_local + self.drift)
            }
        }
    }

    pub fn drift(&self) -> Signed<Duration> {
        self.drift
    }
}

fn duration_since(a: Instant, b: Instant) -> Signed<Duration> {
    if a > b {
        Signed::Pos(a.duration_since(b))
    } else {
        Signed::Neg(b.duration_since(a))
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub enum ClockMessage {
    Elapsed(Signed<Duration>),
    NetworkAnalysis(NetworkAnalysisMessage),
}

#[derive(Debug)]
enum ClockState {
    Synchronizing,
    Start {
        at: Instant,
        unacked: HashSet<SocketAddr>,
        sync_start: Interval,
    },
}

#[derive(Debug)]
struct NetworkQuality {
    rtts: BTreeMap<Instant, Duration>,
    outgoing: HashMap<u64, Instant>,
    pong_queue: VecDeque<(u64, Instant)>,
    ping_interval: Interval,
}

impl Default for NetworkQuality {
    fn default() -> Self {
        NetworkQuality {
            outgoing: Default::default(),
            ping_interval: Interval::new(Duration::from_millis(100)),
            pong_queue: Default::default(),
            rtts: Default::default(),
        }
    }
}

impl NetworkQuality {
    fn message(&mut self) -> Option<NetworkAnalysisMessage> {
        use NetworkAnalysisMessage::*;

        if let Some((data, received_at)) = self.pong_queue.pop_front() {
            return Some(Pong(data, received_at.elapsed()));
        }
        if self.ping_interval.is_time() {
            let id = rand::thread_rng().gen();
            self.outgoing.insert(id, Instant::now());
            return Some(Ping(id));
        }
        None
    }

    fn receive_message(&mut self, message: NetworkAnalysisMessage) {
        use NetworkAnalysisMessage::*;
        self.remove_old_data();

        match message {
            Ping(data) => {
                self.pong_queue.push_back((data, Instant::now()));
            }
            Pong(data, remote_processing_time) => {
                let sent_at = match self.outgoing.remove(&data) {
                    Some(s) => s,
                    None => return,
                };
                self.rtts
                    .insert(Instant::now(), sent_at.elapsed() - remote_processing_time);
            }
        }
    }

    fn remove_old_data(&mut self) {
        if self.rtts.len() <= 10 {
            return;
        }

        while self.rtts.len() > 10 {
            let front = self.rtts.keys().next().unwrap().clone();
            self.rtts.remove(&front);
        }

        let keep_after = match self.rtts.range(..).next() {
            Some((at, _)) => at,
            None => return,
        };
        let remove = self
            .outgoing
            .iter()
            .filter(|(_, at)| at >= &keep_after)
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
        for id in remove {
            self.outgoing.remove(&id);
        }
    }

    fn average_rtt(&self) -> Option<Duration> {
        if self.rtts.len() < 3 {
            return None;
        }
        Some(self.rtts.values().sum::<Duration>() / self.rtts.len() as u32)
    }

    fn worst_case_rtt(&self) -> Option<Duration> {
        if self.rtts.len() < 5 {
            return None;
        }
        self.rtts.values().max().cloned()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub enum NetworkAnalysisMessage {
    Ping(u64),
    Pong(u64, Duration),
}
