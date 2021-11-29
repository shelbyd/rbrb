use crate::{
    time::SharedClock, Frame, Interval, NonBlockingSocket, PlayerId, Session, SessionPlugin,
};

use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

#[derive(Default)]
pub struct SessionBuilder {
    remote_players: Vec<SocketAddr>,
    local_player: Option<PlayerId>,
    step_size: Option<Duration>,
    default_inputs: Option<Vec<u8>>,
    socket: Option<Box<dyn NonBlockingSocket>>,
}

impl SessionBuilder {
    pub fn remote_players(mut self, players: &[SocketAddr]) -> Self {
        self.remote_players = players.to_vec();
        self
    }

    pub fn local_player(mut self, index: PlayerId) -> Self {
        self.local_player = Some(index);
        self
    }

    pub fn step_size(mut self, size: Duration) -> Self {
        self.step_size = Some(size);
        self
    }

    pub fn default_inputs(mut self, inputs: Vec<u8>) -> Self {
        self.default_inputs = Some(inputs);
        self
    }

    pub fn with_socket(mut self, socket: impl NonBlockingSocket + 'static) -> Self {
        self.socket = Some(Box::new(socket));
        self
    }

    pub fn start(self) -> Result<Session, String> {
        let local_id = self.local_player.ok_or("must provide local_player")?;

        let remote_players = self
            .remote_players
            .iter()
            .enumerate()
            .map(|(i, &addr)| {
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
            inputs: crate::InputStorage::with_default(
                self.default_inputs.ok_or("must provide default_inputs")?,
            ),
            host_at: Duration::ZERO,
            step_size: self.step_size.ok_or("must provide step_size")?,
            local_id,
            socket: self.socket.ok_or("must provide socket")?,
            player_addresses: remote_players,
            unconfirmed: Frame(1),
            remote_unconfirmed: Default::default(),
            send_interval: Interval::new(Duration::from_millis(50)),
            shared_clock: SharedClock::among_remotes(self.remote_players.iter().cloned()),
            plugins: {
                [
                    Box::new(crate::plugin::WarnRemoteMismatchedChecksum::with_addrs(
                        self.remote_players.iter().cloned(),
                    )) as Box<dyn SessionPlugin>,
                ]
                .into_iter()
                .map(|p| (p.id().to_owned(), p))
                .collect()
            },
        })
    }
}
