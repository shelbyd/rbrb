use crate::{PlayerInputs, SerializedInput, SerializedState};

use std::{ops::ControlFlow, time::Duration};

pub trait RequestHandler {
    type Break;

    fn handle_request(&mut self, request: Request) -> ControlFlow<Self::Break>;
}

impl<F, M> RequestHandler for F
where
    F: FnMut(Request) -> M,
    M: MaybeMessage,
{
    type Break = M::Message;

    fn handle_request(&mut self, request: Request) -> ControlFlow<Self::Break> {
        if let Some(m) = self(request).as_message() {
            ControlFlow::Break(m)
        } else {
            ControlFlow::Continue(())
        }
    }
}

pub trait MaybeMessage {
    type Message;

    fn as_message(self) -> Option<Self::Message>;
}

impl<M> MaybeMessage for ControlFlow<M> {
    type Message = M;

    fn as_message(self) -> Option<Self::Message> {
        match self {
            ControlFlow::Break(m) => Some(m),
            ControlFlow::Continue(()) => None,
        }
    }
}

impl<M> MaybeMessage for Option<M> {
    type Message = M;

    fn as_message(self) -> Option<Self::Message> {
        self
    }
}

impl MaybeMessage for () {
    // TODO(shelbyd): Should be never (!).
    type Message = ();

    fn as_message(self) -> Option<Self::Message> {
        None
    }
}

pub trait ControlFlowExt {
    type Break;

    fn always<R>(self, f: impl FnOnce() -> R) -> ControlFlow<Self::Break, R>;
    fn map_break<R>(self, f: impl FnOnce(Self::Break) -> R) -> ControlFlow<R>;
}

impl<B> ControlFlowExt for ControlFlow<B> {
    type Break = B;

    fn always<R>(self, f: impl FnOnce() -> R) -> ControlFlow<B, R> {
        let ret = f();
        self?;
        ControlFlow::Continue(ret)
    }

    fn map_break<R>(self, f: impl FnOnce(B) -> R) -> ControlFlow<R> {
        match self {
            ControlFlow::Break(b) => ControlFlow::Break(f(b)),
            ControlFlow::Continue(c) => ControlFlow::Continue(c),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Request<'s> {
    SaveTo(&'s mut SerializedState),
    LoadFrom(&'s [u8]),
    #[non_exhaustive]
    Advance {
        amount: Duration,
        inputs: PlayerInputs,
        confirmed: Confirmation,
        current_frame: u32,
    },
    CaptureLocalInput(&'s mut SerializedInput),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Confirmation {
    Unconfirmed,
    First,
    Subsequent,
}
