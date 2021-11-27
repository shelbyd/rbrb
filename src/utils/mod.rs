use serde::*;
use std::{
    cmp::Ordering,
    iter::Sum,
    ops::{Add, Div, Neg, Sub},
    time::Duration,
};

pub fn div_duration(numerator: Duration, denominator: Duration) -> (u32, Duration) {
    let mut min = 0;
    let mut max = core::u32::MAX;

    while max - min > 1 {
        let mid = min + (max - min) / 2;
        match (denominator * mid).cmp(&numerator) {
            Ordering::Equal => return (mid, Duration::ZERO),
            Ordering::Greater => {
                max = mid;
            }
            Ordering::Less => {
                min = mid;
            }
        }
    }
    (min, numerator - (denominator * min))
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
pub enum Signed<T> {
    Pos(T),
    Neg(T),
}

impl<T> Signed<T> {
    pub fn abs(self) -> T {
        match self {
            Signed::Pos(t) => t,
            Signed::Neg(t) => t,
        }
    }

    #[allow(dead_code)]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Signed<U> {
        match self {
            Signed::Pos(t) => Signed::Pos(f(t)),
            Signed::Neg(t) => Signed::Neg(f(t)),
        }
    }

    #[allow(dead_code)]
    pub fn stretch<U>(self, other: U) -> Signed<T::Output>
    where
        T: Add<U>,
    {
        self.map(|t| t + other)
    }

    #[allow(dead_code)]
    pub fn add_to<U, R>(self, other: U) -> R
    where
        U: Add<T, Output = R> + Sub<T, Output = R>,
    {
        match self {
            Signed::Pos(t) => other + t,
            Signed::Neg(t) => other - t,
        }
    }

    pub fn sub_from<U, R>(self, other: U) -> R
    where
        U: Add<T, Output = R> + Sub<T, Output = R>,
    {
        match self {
            Signed::Pos(t) => other - t,
            Signed::Neg(t) => other + t,
        }
    }

    pub fn pos(self) -> Option<T> {
        match self {
            Signed::Pos(t) => Some(t),
            Signed::Neg(_) => None,
        }
    }
}

impl<T> Default for Signed<T>
where
    T: Default,
{
    fn default() -> Self {
        Signed::Pos(Default::default())
    }
}

impl<T> From<T> for Signed<T> {
    fn from(t: T) -> Self {
        Signed::Pos(t)
    }
}

impl<T, U, R> Add<Signed<U>> for Signed<T>
where
    T: Sub<U, Output = R> + Add<U, Output = R> + PartialOrd<U>,
    U: Sub<T, Output = R>,
{
    type Output = Signed<R>;

    fn add(self, other: Signed<U>) -> Signed<R> {
        match (self, other) {
            (Signed::Pos(t), Signed::Pos(u)) => Signed::Pos(t + u),
            (Signed::Neg(t), Signed::Neg(u)) => Signed::Neg(t + u),
            (Signed::Pos(t), Signed::Neg(u)) => {
                if t > u {
                    Signed::Pos(t - u)
                } else {
                    Signed::Neg(u - t)
                }
            }
            (Signed::Neg(t), Signed::Pos(u)) => {
                if t > u {
                    Signed::Neg(t - u)
                } else {
                    Signed::Pos(u - t)
                }
            }
        }
    }
}

impl<T, U, R> Sub<Signed<U>> for Signed<T>
where
    T: Sub<U, Output = R> + Add<U, Output = R> + PartialOrd<U>,
    U: Sub<T, Output = R>,
{
    type Output = Signed<R>;

    fn sub(self, other: Signed<U>) -> Signed<R> {
        self + (-other)
    }
}

impl<T, U, R> Div<U> for Signed<T>
where
    T: Div<U, Output = R>,
{
    type Output = Signed<R>;

    fn div(self, other: U) -> Signed<R> {
        self.map(|t| t / other)
    }
}

impl<T> Neg for Signed<T> {
    type Output = Signed<T>;

    fn neg(self) -> Self {
        match self {
            Signed::Pos(t) => Signed::Neg(t),
            Signed::Neg(t) => Signed::Pos(t),
        }
    }
}

impl<T, U> PartialEq<Signed<U>> for Signed<T>
where
    T: PartialEq<U>,
{
    fn eq(&self, other: &Signed<U>) -> bool {
        match (self, other) {
            (Signed::Pos(t), Signed::Pos(u)) | (Signed::Neg(t), Signed::Neg(u)) => t.eq(u),
            _ => false,
        }
    }
}

impl<T> Eq for Signed<T> where T: Eq {}

impl<T, U> PartialOrd<Signed<U>> for Signed<T>
where
    T: PartialOrd<U>,
{
    fn partial_cmp(&self, other: &Signed<U>) -> Option<Ordering> {
        match (self, other) {
            (Signed::Pos(t), Signed::Pos(u)) => t.partial_cmp(u),
            (Signed::Neg(t), Signed::Neg(u)) => t.partial_cmp(u).map(Ordering::reverse),
            (Signed::Pos(_), Signed::Neg(_)) => Some(Ordering::Greater),
            (Signed::Neg(_), Signed::Pos(_)) => Some(Ordering::Less),
        }
    }
}

impl<T> Ord for Signed<T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Signed::Pos(t), Signed::Pos(u)) => t.cmp(u),
            (Signed::Neg(t), Signed::Neg(u)) => t.cmp(u).reverse(),
            (Signed::Pos(_), Signed::Neg(_)) => Ordering::Greater,
            (Signed::Neg(_), Signed::Pos(_)) => Ordering::Less,
        }
    }
}

impl<T> Sum for Signed<T>
where
    T: Default,
    Signed<T>: Add<Output = Signed<T>>,
{
    fn sum<I>(iter: I) -> Self
    where
        I: Iterator<Item = Self>,
    {
        let mut value = Signed::<T>::default();
        for item in iter {
            value = value + item;
        }
        value
    }
}
