use super::{BasicUdpSocket, NonBlockingSocket};

use rand::{rngs::SmallRng, Rng, SeedableRng};
use rand_distr::{Distribution, Poisson};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

pub struct BadSocket<S: NonBlockingSocket> {
    socket: S,

    rng: SmallRng,
    success_chance: f64,
    lag: Poisson<f32>,

    send_delays: BTreeMap<Instant, (Vec<u8>, SocketAddr)>,
    recv_delays: BTreeMap<Instant, (SocketAddr, Vec<u8>)>,

    owned_for_lifetime: Option<(SocketAddr, Vec<u8>)>,
}

impl BadSocket<BasicUdpSocket> {
    pub fn bind(port: u16) -> std::io::Result<Self> {
        Ok(Self::new(BasicUdpSocket::bind(port)?))
    }
}

impl<S: NonBlockingSocket> BadSocket<S> {
    pub fn new(socket: S) -> Self {
        Self {
            socket,
            rng: SmallRng::from_entropy(),
            success_chance: 0.4,
            lag: Poisson::new(100.).unwrap(),
            send_delays: Default::default(),
            recv_delays: Default::default(),
            owned_for_lifetime: None,
        }
    }

    fn packet_behavior(&mut self) -> PacketBehavior {
        if !self.rng.gen_bool(self.success_chance) {
            PacketBehavior::Drop
        } else {
            let lag = self.lag.sample(&mut self.rng);
            PacketBehavior::Delay(Duration::from_millis(lag as u64))
        }
    }
}

enum PacketBehavior {
    Drop,
    Delay(Duration),
}

fn next_ready<T>(map: &mut BTreeMap<Instant, T>) -> Option<T> {
    let (&first_at, _) = map.range(..).next()?;
    if first_at <= Instant::now() {
        map.remove(&first_at)
    } else {
        None
    }
}

impl<S: NonBlockingSocket> NonBlockingSocket for BadSocket<S> {
    fn send(&mut self, message: &[u8], addr: SocketAddr) {
        while let Some((message, addr)) = next_ready(&mut self.send_delays) {
            self.socket.send(&message, addr);
        }

        match self.packet_behavior() {
            PacketBehavior::Drop => {}
            PacketBehavior::Delay(amount) => {
                self.send_delays
                    .insert(Instant::now() + amount, (message.to_vec(), addr));
            }
        }
    }

    fn recv(&mut self) -> Option<(SocketAddr, &[u8])> {
        loop {
            if let Some(packet) = next_ready(&mut self.recv_delays) {
                self.owned_for_lifetime = Some(packet);
                return self
                    .owned_for_lifetime
                    .as_ref()
                    .map(|(a, v)| (*a, v.as_slice()));
            }
            match self.packet_behavior() {
                PacketBehavior::Drop => {
                    self.socket.recv()?;
                }
                PacketBehavior::Delay(amount) => {
                    let (from, bytes) = self.socket.recv()?;
                    self.recv_delays
                        .insert(Instant::now() + amount, (from, bytes.to_vec()));
                }
            }
        }
    }
}
