//! Tiny, dependency-free, seedable PRNG (SplitMix64). Identical seed =>
//! identical stream on every platform and every toolchain version, which is
//! what deterministic-simulation reproducibility requires.

#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    /// Standard normal via Box-Muller.
    pub fn next_normal(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// Uniform integer in [0, n). n must be > 0.
    pub fn gen_range(&mut self, n: usize) -> usize {
        (self.next_u64() % (n as u64)) as usize
    }

    /// Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        if v.len() < 2 {
            return;
        }
        for i in (1..v.len()).rev() {
            let j = self.gen_range(i + 1);
            v.swap(i, j);
        }
    }
}

/// Derive a child seed from a base seed and a string tag (FNV-1a mixing).
pub fn derive_seed(base: u64, tag: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ base;
    for b in tag.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // one SplitMix round to spread low-entropy inputs
    SplitMix64::new(h).next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn f64_in_unit_interval() {
        let mut r = SplitMix64::new(7);
        for _ in 0..10_000 {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn derive_seed_differs_by_tag() {
        assert_ne!(derive_seed(1, "a"), derive_seed(1, "b"));
        assert_eq!(derive_seed(1, "a"), derive_seed(1, "a"));
    }

    #[test]
    fn shuffle_is_permutation() {
        let mut r = SplitMix64::new(3);
        let mut v: Vec<usize> = (0..20).collect();
        r.shuffle(&mut v);
        let mut s = v.clone();
        s.sort();
        assert_eq!(s, (0..20).collect::<Vec<_>>());
    }
}
