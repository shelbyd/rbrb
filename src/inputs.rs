use crate::{Frame, PlayerId};

use derive_more::*;
use std::collections::{BTreeMap, HashMap};

pub type SerializedInput = Vec<u8>;

pub(crate) struct InputStorage {
    inputs: HashMap<PlayerId, SparseInputs>,
    default: Vec<u8>,
}

impl InputStorage {
    pub fn with_default(default: Vec<u8>) -> Self {
        InputStorage {
            inputs: Default::default(),
            default,
        }
    }

    pub fn capture_into(&mut self, frame: Frame, local_id: PlayerId) -> Option<&mut Vec<u8>> {
        if frame == Frame(0) {
            return None;
        }

        self.sparse_mut(local_id).capture_into(frame)
    }

    pub fn sparse_mut(&mut self, id: PlayerId) -> &mut SparseInputs {
        self.inputs.entry(id).or_insert_with(|| {
            let mut i = SparseInputs::default();
            i.insert(Frame(0), self.default.clone());
            i
        })
    }

    pub fn at_frame(&self, frame: Frame) -> Option<PlayerInputs> {
        let mut result = PlayerInputs::default();
        for (player, inputs) in &self.inputs {
            if let Some(input) = inputs.at(frame) {
                result.map.insert(*player, input.map(Clone::clone));
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
        self.sparse_mut(player_id)
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
    fn at(&self, frame: Frame) -> Option<Confirmation<&SerializedInput>> {
        let (before_frame, before_value) = self.map.range(..=frame).next_back()?;
        let after = self.map.range(frame..).next();
        match after {
            _ if *before_frame == frame => Some(Confirmation::Confirmed(before_value)),
            Some(_) => Some(Confirmation::Confirmed(before_value)),
            None => Some(Confirmation::Unconfirmed(before_value)),
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

#[derive(Debug, Clone)]
pub struct PlayerInputs<T = Confirmation<SerializedInput>> {
    map: HashMap<PlayerId, T>,
}

impl PlayerInputs {
    pub fn is_fully_confirmed(&self, remote_count: usize) -> bool {
        self.is_fully_populated(remote_count) && self.map.values().all(Confirmation::is_confirmed)
    }

    pub fn is_fully_populated(&self, remote_count: usize) -> bool {
        let should_have = remote_count + 1;
        let len = self.map.len();
        assert!(len <= should_have);
        len == should_have
    }
}

impl<T> PlayerInputs<Confirmation<T>> {
    pub fn deep_map<U>(self, mut f: impl FnMut(T) -> U) -> PlayerInputs<Confirmation<U>> {
        PlayerInputs {
            map: self
                .map
                .into_iter()
                .map(move |(k, v)| (k, v.map(|v| f(v))))
                .collect(),
        }
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

impl<T> Default for PlayerInputs<T> {
    fn default() -> Self {
        PlayerInputs {
            map: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation<T> {
    Confirmed(T),
    Unconfirmed(T),
}

impl<T> Confirmation<T> {
    pub fn into_inner(self) -> T {
        match self {
            Confirmation::Confirmed(t) => t,
            Confirmation::Unconfirmed(t) => t,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Confirmation<U> {
        match self {
            Confirmation::Confirmed(t) => Confirmation::Confirmed(f(t)),
            Confirmation::Unconfirmed(t) => Confirmation::Unconfirmed(f(t)),
        }
    }

    pub fn is_confirmed(&self) -> bool {
        match self {
            Confirmation::Confirmed(_) => true,
            Confirmation::Unconfirmed(_) => false,
        }
    }
}
