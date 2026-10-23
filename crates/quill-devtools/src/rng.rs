//! A small reproducible random number generator (SplitMix64). Demo data only:
//! this is not suitable for anything security-related.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`. `n` must be non-zero.
    pub fn below(&mut self, n: usize) -> usize {
        // The modulo bias is irrelevant for demo data.
        usize::try_from(self.next_u64() % n as u64).unwrap_or(0)
    }

    /// A number in `lo..=hi`.
    pub fn between(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    /// True with probability `percent` / 100.
    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Up to `n` distinct items, in random order.
    pub fn pick_distinct<T: Clone>(&mut self, items: &[T], n: usize) -> Vec<T> {
        let mut pool: Vec<T> = items.to_vec();
        let mut out = Vec::new();
        while out.len() < n && !pool.is_empty() {
            out.push(pool.swap_remove(self.below(pool.len())));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproducible_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
            let x = a.between(3, 5);
            assert!((3..=5).contains(&x));
            b.between(3, 5);
        }
        let picked = a.pick_distinct(&[1, 2, 3], 5);
        assert_eq!(picked.len(), 3);
    }
}
