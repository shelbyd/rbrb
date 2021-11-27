use crate::{utils::Signed, NonBlockingSocket};
use bytesize::*;
use std::{net::SocketAddr, time::Duration};

mod historical;
use historical::*;

pub struct NetworkStats {
    pub drift: Signed<Duration>,
    pub elapsed: Signed<Duration>,
    pub socket: Option<SocketStats>,
}

pub struct SocketStats {
    pub outgoing_bytes: ByteSize,
    pub incoming_bytes: ByteSize,
}

pub struct BandwidthRecordingSocket<S: NonBlockingSocket> {
    socket: S,
    incoming_bytes: Historical,
    outgoing_bytes: Historical,
}

impl<S: NonBlockingSocket> BandwidthRecordingSocket<S> {
    pub fn new(socket: S) -> Self {
        BandwidthRecordingSocket {
            socket,
            incoming_bytes: Historical::over_secs(3),
            outgoing_bytes: Historical::over_secs(3),
        }
    }

    fn clean_old(&mut self) {
        self.incoming_bytes.clean();
        self.outgoing_bytes.clean();
    }
}

impl<S: NonBlockingSocket> NonBlockingSocket for BandwidthRecordingSocket<S> {
    fn send(&mut self, message: &[u8], addr: SocketAddr) {
        self.clean_old();

        self.outgoing_bytes.increment(message.len() as u64);
        self.socket.send(message, addr);
    }

    fn recv(&mut self) -> Option<(SocketAddr, &[u8])> {
        self.clean_old();

        let (from, m) = self.socket.recv()?;
        self.incoming_bytes.increment(m.len() as u64);
        Some((from, m))
    }

    fn stats(&self) -> Option<SocketStats> {
        Some(SocketStats {
            incoming_bytes: ByteSize(self.incoming_bytes.avg_per_sec()),
            outgoing_bytes: ByteSize(self.outgoing_bytes.avg_per_sec()),
        })
    }
}
