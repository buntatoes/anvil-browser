// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! A tiny seeded random number generator for the stress tests.
//!
//! Xorshift with a multiply on the way out. It's not for anything that needs
//! real randomness; it just has to be repeatable from a seed.

/// Seeded generator.
#[derive(Clone, Debug)]
pub struct Prng {
    state: u64,
}

impl Prng {
    /// A generator from `seed`. Any seed works; the state is never zero,
    /// since xorshift would get stuck there.
    pub fn new(seed: u64) -> Prng {
        // Spread the seed's bits so nearby seeds start far apart.
        let mut s = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        s = (s ^ (s >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        s = (s ^ (s >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        s ^= s >> 31;
        Prng {
            state: if s == 0 { 1 } else { s },
        }
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0..n` (`n` must be above 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// True with chance `percent` in 100.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Prng::new(42);
        let mut b = Prng::new(42);
        let mut c = Prng::new(43);
        let xs: Vec<u64> = (0..10).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..10).map(|_| b.next_u64()).collect();
        let zs: Vec<u64> = (0..10).map(|_| c.next_u64()).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
        let mut z = Prng::new(0);
        assert_ne!(z.next_u64(), 0);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Prng::new(7);
        let mut seen = [false; 5];
        for _ in 0..1000 {
            let v = r.below(5);
            seen[v] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }
}
