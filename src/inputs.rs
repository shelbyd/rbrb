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
        inputs.capture_into(frame)
    }

    pub fn at_frame(&self, frame: Frame) -> Option<PlayerInputs> {
        let mut result = PlayerInputs::default();
        for (player, inputs) in &self.inputs {
            if let Some(input) = inputs.at(frame) {
                result.map.insert(*player, input.clone());
            }
        }
        if result.map.len() == 0 {
            None
        } else {
            Some(result)
        }
    }

    pub fn player_since_frame(
        &mut self,
        player_id: PlayerId,
        frame: Frame,
    ) -> BTreeMap<Frame, SerializedInput> {
        self.inputs
            .entry(player_id)
            .or_default()
            .iter()
            .rev()
            .take_while(|(&k, _)| k >= frame)
            .map(|(&k, v)| (k, v.clone()))
            .collect()
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

#[derive(Deref, DerefMut)]
pub(crate) struct SparseInputs {
    #[deref]
    #[deref_mut]
    map: BTreeMap<Frame, SerializedInput>,
    next_compact: Frame,
}

impl SparseInputs {
    fn at(&self, frame: Frame) -> Option<&SerializedInput> {
        let (before_frame, before_value) = self.map.range(..=frame).next_back()?;
        let after = self.map.range(frame..).next();
        match after {
            _ if *before_frame == frame => Some(before_value),
            Some(_) => Some(before_value),
            None => None,
        }
    }

    fn capture_into(&mut self, frame: Frame) -> Option<&mut SerializedInput> {
        if self.map.contains_key(&frame) {
            return None;
        }

        self.compact();
        Some(self.map.entry(frame).or_default())
    }

    fn compact(&mut self) -> Option<()> {
        loop {
            let mut at_or_after = self.map.range(self.next_compact..);
            let (next_frame, next_input) = at_or_after.next()?;
            let next_frame = *next_frame;

            // Should not compact the last frame. We could not provide confident values for frames
            // that have been captured if they were the same as previous frames.
            at_or_after.next()?;

            if let Some((_, before)) = self.map.range(..self.next_compact).next_back() {
                if before == next_input {
                    self.map.remove(&next_frame);
                }
            } else {
                debug_assert_eq!(self.next_compact, Frame(0));
            }
            self.next_compact = next_frame + 1;
        }
    }
}

impl Default for SparseInputs {
    fn default() -> Self {
        SparseInputs {
            map: Default::default(),
            next_compact: Frame(0),
        }
    }
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
