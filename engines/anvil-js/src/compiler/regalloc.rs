// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Register allocation.
//!
//! Registers are handed out like a stack: locals when their scope starts,
//! temporaries while an expression is built, and everything above a mark
//! is given back at once with [`RegAlloc::release_to`]. That keeps call
//! arguments contiguous for free (allocate them last, in order) and the
//! high-water mark is the function's register count.

use crate::anvil_js::bytecode::Reg;

/// Most registers one function can address (registers are 16-bit).
pub const MAX_REGS: u32 = 65_535;

/// A stack of registers for one function.
#[derive(Debug, Default)]
pub struct RegAlloc {
    next: u32,
    high: u32,
}

/// Asked for more registers than a function can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRegisters;

impl RegAlloc {
    /// An allocator whose first `fixed` registers are already taken (the
    /// parameters).
    pub fn with_fixed(fixed: u32) -> RegAlloc {
        RegAlloc {
            next: fixed,
            high: fixed,
        }
    }

    /// One more register.
    pub fn alloc(&mut self) -> Result<Reg, OutOfRegisters> {
        self.alloc_n(1)
    }

    /// `n` consecutive registers; returns the first. With `n == 0` this is
    /// the next free register, which is fine as an empty argument list.
    pub fn alloc_n(&mut self, n: u32) -> Result<Reg, OutOfRegisters> {
        let first = self.next;
        let end = first.checked_add(n).ok_or(OutOfRegisters)?;
        if end > MAX_REGS || (n == 0 && first >= MAX_REGS) {
            return Err(OutOfRegisters);
        }
        self.next = end;
        self.high = self.high.max(end);
        Ok(first as Reg)
    }

    /// Where the stack is now, for [`RegAlloc::release_to`].
    pub fn mark(&self) -> u32 {
        self.next
    }

    /// Give back every register handed out since `mark`.
    pub fn release_to(&mut self, mark: u32) {
        debug_assert!(mark <= self.next);
        self.next = mark.min(self.next);
    }

    /// Registers the function needs in all.
    pub fn high_water(&self) -> u32 {
        self.high
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_order_and_high_water() {
        let mut r = RegAlloc::with_fixed(2);
        assert_eq!(r.alloc(), Ok(2));
        let m = r.mark();
        assert_eq!(r.alloc_n(3), Ok(3));
        assert_eq!(r.high_water(), 6);
        r.release_to(m);
        assert_eq!(r.alloc(), Ok(3));
        assert_eq!(r.high_water(), 6);
    }

    #[test]
    fn running_out_is_an_error() {
        let mut r = RegAlloc::with_fixed(MAX_REGS - 1);
        assert_eq!(r.alloc(), Ok((MAX_REGS - 1) as Reg));
        assert_eq!(r.alloc(), Err(OutOfRegisters));
        assert_eq!(r.alloc_n(0), Err(OutOfRegisters));
        assert_eq!(r.alloc_n(u32::MAX), Err(OutOfRegisters));
    }
}
