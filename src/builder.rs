use crate::{socket::BasicUdpSocket, Frame, NonBlockingSocket, PlayerId, Session};

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct SessionBuilder {
    remote_players: Vec<SocketAddr>,
    local_player: Option<(PlayerId, u16)>,
    step_size: Option<Duration>,
    socket: Option<Box<dyn NonBlockingSocket>>,
}

impl SessionBuilder {
    pub fn remote_players(mut self, players: &[SocketAddr]) -> Self {
        self.remote_players = players.to_vec();
        self
    }

    pub fn local_player(mut self, index: PlayerId, port: u16) -> Self {
        self.local_player = Some((index, port));
        self
    }

    pub fn step_size(mut self, size: Duration) -> Self {
        self.step_size = Some(size);
        self
    }

    pub fn with_socket(mut self, socket: impl NonBlockingSocket + 'static) -> Self {
        self.socket = Some(Box::new(socket));
        self
    }

    pub fn start(self) -> Result<Session, String> {
        let (local_id, port) = self.local_player.ok_or("must provide local_player")?;

        let remote_players = self
            .remote_players
            .into_iter()
            .enumerate()
            .map(|(i, addr)| {
                let i = i as u16;
                if i >= local_id {
                    (addr, i + 1)
                } else {
                    (addr, i)
                }
            })
            .collect();

        Ok(Session {
            confirmed_states: BTreeMap::default(),
            inputs: crate::InputStorage::default(),
            host_at: Duration::ZERO,
            started_at: Instant::now(),
            step_size: self.step_size.ok_or("must provide step_size")?,
            local_id,
            socket: self
                .socket
                .unwrap_or_else(|| Box::new(BasicUdpSocket::bind(port).unwrap())),
            remote_players,
            unconfirmed: Frame(1),
            last_send: None,
            send_every: Duration::from_millis(50),
        })
    }
}
