use std::{cmp::Ordering, time::Duration};

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
