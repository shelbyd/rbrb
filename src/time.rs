use rand::Rng;
use serde::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    net::SocketAddr,
    ops::{Add, Sub},
    time::{Duration, Instant},
};

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
}

#[derive(Debug)]
pub struct SharedClock {
    state: ClockState,
    remotes: HashMap<SocketAddr, NetworkQuality>,
    queue: VecDeque<(SocketAddr, ClockMessage)>,
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

        match &mut self.state {
            ClockState::Start {
                unacked,
                at,
                sync_start,
                ..
            } => {
                if !sync_start.is_time() {
                    return None;
                }

                let message = ClockMessage::Start(duration_since(*at, Instant::now()));
                self.queue
                    .extend(unacked.iter().map(|addr| (*addr, message.clone())));
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

            ClockMessage::Start(dur) => {
                log::info!("remote {} starting in {:?}", from, dur);

                if let Some(rtt) = self.remotes[&from].average_rtt() {
                    let delta = dur.stretch(rtt / 2);
                    let start_at = delta.add_to(Instant::now());

                    if !self.update_start_time(start_at) {
                        match &mut self.state {
                            ClockState::Start { unacked, at, .. } => {
                                unacked.remove(&from);
                                if rand::thread_rng().gen() {
                                    let message =
                                        ClockMessage::Start(duration_since(*at, Instant::now()));
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

    fn update_start_time(&mut self, new_at: Instant) -> bool {
        match &self.state {
            ClockState::Synchronizing => {}
            ClockState::Start { at, .. } => {
                if *at < Instant::now() {
                    return false;
                }
                if duration_since(*at, new_at).abs() < Duration::from_millis(10) {
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
        match self.state {
            ClockState::Synchronizing => None,
            ClockState::Start { at, .. } => Instant::now().checked_duration_since(at),
        }
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
    Start(Signed<Duration>),
    NetworkAnalysis(NetworkAnalysisMessage),
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub enum Signed<T> {
    Pos(T),
    Neg(T),
}

impl<T> Signed<T> {
    fn abs(self) -> T {
        match self {
            Signed::Pos(t) => t,
            Signed::Neg(t) => t,
        }
    }

    fn map<U>(self, f: impl FnOnce(T) -> U) -> Signed<U> {
        match self {
            Signed::Pos(t) => Signed::Pos(f(t)),
            Signed::Neg(t) => Signed::Neg(f(t)),
        }
    }

    fn stretch<U>(self, other: U) -> Signed<T::Output>
    where
        T: Add<U>,
    {
        self.map(|t| t + other)
    }

    fn add_to<U, R>(self, other: U) -> R
    where
        U: Add<T, Output = R> + Sub<T, Output = R>,
    {
        match self {
            Signed::Pos(t) => other + t,
            Signed::Neg(t) => other - t,
        }
    }
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
