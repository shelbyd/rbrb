use crate::{Frame, PlayerId};

use derive_more::*;
use std::collections::{BTreeMap, HashMap};

pub type SerializedInput = Vec<u8>;

#[derive(Default)]
pub(crate) struct InputStorage {
    inputs: HashMap<PlayerId, SparseInputs>,
}

impl InputStorage {
    pub fn capture_into(&mut self, frame: Frame, local_id: PlayerId) -> Option<&mut Vec<u8>> {
        let inputs = self.inputs.entry(local_id).or_default();
        if inputs.contains_key(&frame) {
            return None;
        }

        Some(inputs.entry(frame).or_default())
    }

    pub fn at_frame(&self, frame: Frame) -> Option<PlayerInputs> {
        let mut inputs = PlayerInputs::default();
        for (player, remote_inputs) in &self.inputs {
            if let Some(input) = remote_inputs.get(&frame) {
                inputs.map.insert(*player, input.clone());
            }
        }
        if inputs.map.len() == 0 {
            None
        } else {
            Some(inputs)
        }
    }

    pub fn player_since_frame(
        &mut self,
        player_id: PlayerId,
        _frame: Frame,
    ) -> BTreeMap<Frame, SerializedInput> {
        // TODO(shelbyd): Filter by frame.
        self.inputs.entry(player_id).or_default().clone()
    }

    pub fn merge_remote(&mut self, player: PlayerId, map: BTreeMap<Frame, SerializedInput>) {
        for (frame, input) in map {
            self.inputs
                .entry(player)
                .or_default()
                .entry(frame)
                .or_insert(input);
        }
    }
}

#[derive(Default, Deref, DerefMut)]
pub(crate) struct SparseInputs {
    map: BTreeMap<Frame, SerializedInput>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerInputs<T = SerializedInput> {
    map: HashMap<PlayerId, T>,
}

impl PlayerInputs {
    pub fn is_complete(&self, remote_count: usize) -> bool {
        let should_have = remote_count + 1;
        let len = self.map.len();
        assert!(len <= should_have);
        len == should_have
    }
}

impl<T> PlayerInputs<T> {
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> PlayerInputs<U> {
        PlayerInputs {
            map: self.map.into_iter().map(|(k, v)| (k, f(v))).collect(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PlayerId, &T)> {
        self.map.iter()
    }

    pub fn get(&self, player: &PlayerId) -> Option<&T> {
        self.map.get(player)
    }
}
