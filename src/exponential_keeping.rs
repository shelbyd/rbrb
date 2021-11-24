use std::collections::HashSet;

pub(crate) fn kept_set(total: u32) -> HashSet<u32> {
    let mut set = naive_kept_set(total);
    if !total.is_power_of_two() {
        set.insert((total / 2).next_power_of_two());
    }
    set
}

fn naive_kept_set(total: u32) -> HashSet<u32> {
    if total == 0 {
        return HashSet::new();
    }
    if total == 1 {
        return vec![0].into_iter().collect();
    }

    kept_set(total / 2)
        .into_iter()
        .map(|n| n * 2)
        .chain([total - 1])
        .collect()
}

#[cfg(test)]
mod tests {
    fn kept_set(total: u32) -> Vec<u32> {
        let mut vec = super::kept_set(total).into_iter().collect::<Vec<_>>();
        vec.sort();
        vec
    }

    #[test]
    fn kept_set_values() {
        assert_eq!(kept_set(2), vec![0, 1]);
        assert_eq!(kept_set(3), vec![0, 1, 2]);
        assert_eq!(kept_set(4), vec![0, 2, 3]);
        assert_eq!(kept_set(6), vec![0, 2, 4, 5]);
        assert_eq!(kept_set(8), vec![0, 4, 6, 7]);
        assert_eq!(kept_set(29), vec![0, 8, 16, 24, 26, 28]);

        assert_eq!(kept_set(9), vec![0, 4, 6, 8]);
        assert_eq!(kept_set(10), vec![0, 4, 8, 9]);
    }

    #[quickcheck_macros::quickcheck]
    fn all_below_included(total: u32, increment: u32) -> bool {
        let larger = total.saturating_add(increment);
        let smaller = kept_set(total);
        kept_set(larger)
            .into_iter()
            .filter(|n| *n < total)
            .all(|n| smaller.contains(&n))
    }

    #[quickcheck_macros::quickcheck]
    fn no_gap_larger(total: u32) -> bool {
        let max_gap = total / 2;
        kept_set(total)
            .windows(2)
            .all(|pair| pair[1] - pair[0] <= max_gap)
    }

    #[quickcheck_macros::quickcheck]
    fn no_fewer(total: u32) -> bool {
        kept_set(total).len() <= kept_set(total.saturating_add(1)).len()
    }
}
