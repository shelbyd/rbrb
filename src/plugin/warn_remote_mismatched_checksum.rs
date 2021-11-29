use lru::LruCache;
use serde::*;
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

use super::SessionPlugin;
use crate::{Frame, Interval};

type ChecksumCache = LruCache<Frame, u64>;

pub struct WarnRemoteMismatchedChecksum {
    addrs: Vec<SocketAddr>,
    checksums: ChecksumCache,
    remote_checksums: BTreeMap<SocketAddr, ChecksumCache>,
    send_every: Interval,
}

impl WarnRemoteMismatchedChecksum {
    pub fn with_addrs(addrs: impl IntoIterator<Item = SocketAddr>) -> Self {
        WarnRemoteMismatchedChecksum {
            addrs: addrs.into_iter().collect(),
            checksums: LruCache::new(1024),
            remote_checksums: BTreeMap::default(),
            send_every: Interval::new(Duration::from_millis(500)),
        }
    }

    fn typed_messages(&mut self) -> Vec<(SocketAddr, Message)> {
        if !self.send_every.is_time() {
            return Vec::new();
        }

        let (frame, checksum) = match self.checksums.iter().max_by_key(|(&frame, _)| frame) {
            Some(l) => l,
            None => return Vec::new(),
        };
        self.addrs
            .iter()
            .map(|a| (*a, Message::FrameChecksum(*frame, *checksum)))
            .collect()
    }

    fn check_frame_match(&mut self, frame: Frame) {
        let ours = match self.checksums.get(&frame) {
            Some(c) => c,
            None => return,
        };

        for (remote, checksums) in &mut self.remote_checksums {
            let theirs = match checksums.pop(&frame) {
                Some(c) => c,
                None => continue,
            };

            assert_eq!(
                *ours, theirs,
                "Checksum mismatch for frame {:?} with remote {}",
                frame, remote
            );
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum Message {
    FrameChecksum(Frame, u64),
}

impl SessionPlugin for WarnRemoteMismatchedChecksum {
    fn id(&self) -> &str {
        "warn_remote_mismatched_checksum"
    }

    fn on_confirmed_frame(&mut self, frame: Frame, serialized: &[u8]) {
        let checksum = seahash::hash(serialized);
        self.checksums.put(frame, checksum);
        self.check_frame_match(frame);
    }

    fn messages(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        self.typed_messages()
            .into_iter()
            .map(|(to, m)| (to, bincode::serialize(&m).unwrap()))
            .collect()
    }

    fn receive(&mut self, from: SocketAddr, message: Vec<u8>) {
        let message = bincode::deserialize(&message).unwrap();
        match message {
            Message::FrameChecksum(frame, checksum) => {
                self.remote_checksums
                    .entry(from)
                    .or_insert_with(|| LruCache::new(1024))
                    .put(frame, checksum);
                self.check_frame_match(frame);
            }
        }
    }
}
