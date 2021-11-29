use std::net::SocketAddr;

use crate::Frame;

mod warn_remote_mismatched_checksum;
pub use warn_remote_mismatched_checksum::*;

pub trait SessionPlugin {
    fn id(&self) -> &str;

    fn on_confirmed_frame(&mut self, _frame: Frame, _serialized: &[u8]) {}

    fn messages(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        Vec::new()
    }
    fn receive(&mut self, _from: SocketAddr, _message: Vec<u8>) {}
}
