use std::{
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
};

use crate::stats::SocketStats;

mod bad;
pub use bad::*;

pub trait NonBlockingSocket {
    fn send(&mut self, message: &[u8], addr: SocketAddr);
    fn recv(&mut self) -> Option<(SocketAddr, &[u8])>;

    fn stats(&self) -> Option<SocketStats> {
        None
    }
}

pub struct BasicUdpSocket {
    socket: UdpSocket,
    buffer: Vec<u8>,
}

impl BasicUdpSocket {
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        Ok(BasicUdpSocket {
            socket,
            buffer: vec![0; 64],
        })
    }
}

impl NonBlockingSocket for BasicUdpSocket {
    fn send(&mut self, message: &[u8], addr: SocketAddr) {
        self.socket.send_to(message, addr).expect("failed to send");
    }

    fn recv(&mut self) -> Option<(SocketAddr, &[u8])> {
        match self.socket.recv_from(&mut self.buffer[..]) {
            Ok((amount, addr)) => {
                if amount == self.buffer.len() {
                    log::info!("doubling receive buffer to {} bytes", self.buffer.len() * 2);
                    self.buffer
                        .extend(std::iter::repeat(0).take(self.buffer.len()));
                }
                Some((addr, &self.buffer[0..amount]))
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => None,
            unhandled => {
                unimplemented!("unhandled: {:?}", unhandled);
            }
        }
    }
}
