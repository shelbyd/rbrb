use std::{
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
};

pub trait NonBlockingSocket {
    fn send(&mut self, message: &[u8], addr: SocketAddr);
    fn recv(&mut self) -> Option<(SocketAddr, &[u8])>;
}

pub(crate) struct BasicUdpSocket {
    socket: UdpSocket,
    buffer: Vec<u8>,
}

impl BasicUdpSocket {
    pub(crate) fn bind(port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        Ok(BasicUdpSocket {
            socket,
            buffer: vec![0; 4096],
        })
    }
}

impl NonBlockingSocket for BasicUdpSocket {
    fn send(&mut self, message: &[u8], addr: SocketAddr) {
        self.socket.send_to(message, addr).expect("failed to send");
    }

    fn recv(&mut self) -> Option<(SocketAddr, &[u8])> {
        match self.socket.recv_from(&mut self.buffer[..]) {
            Ok((amount, addr)) => Some((addr, &self.buffer[0..amount])),
            Err(e) if e.kind() == ErrorKind::WouldBlock => None,
            unhandled => {
                unimplemented!("unhandled: {:?}", unhandled);
            }
        }
    }
}
