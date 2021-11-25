use super::{BasicUdpSocket, NonBlockingSocket};

use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::net::SocketAddr;

pub struct BadSocket {
    socket: BasicUdpSocket,
    rng: SmallRng,
    success_chance: f64,
}

impl BadSocket {
    pub fn bind(port: u16) -> std::io::Result<Self> {
        Ok(Self {
            socket: BasicUdpSocket::bind(port)?,
            rng: SmallRng::from_entropy(),
            success_chance: 0.4,
        })
    }
}

impl NonBlockingSocket for BadSocket {
    fn send(&mut self, message: &[u8], addr: SocketAddr) {
        if self.rng.gen_bool(self.success_chance) {
            self.socket.send(message, addr);
        }
    }

    fn recv(&mut self) -> Option<(SocketAddr, &[u8])> {
        while !self.rng.gen_bool(self.success_chance) {
            self.socket.recv()?;
        }
        self.socket.recv()
    }
}
