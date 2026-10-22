//! Adaptive concurrency of one phase (Go `phaseConcurrency`).
//!
//! A rolling semaphore: `acquire` blocks while `limit()` invoices are in
//! flight, so the next invoice starts the moment a slot frees. The limit
//! starts at the phase's `--workers-<phase>` ceiling. A finished invoice that
//! saw a transient database conflict (serialization failure, deadlock, lost
//! connection that outlived the store's own retries) halves it (never below
//! one) and pauses regrowth for [`COOLDOWN_INVOICES`] finished invoices; after
//! that every [`GROW_EVERY`] clean invoices grow it by one back to the
//! ceiling. Invoices admitted before a halving that report a conflict only
//! restart the cooldown, so one incident does not collapse the limit to 1.

use std::sync::{Condvar, Mutex, MutexGuard};

/// Clean finished invoices that regrow the limit by one.
pub const GROW_EVERY: usize = 20;
/// Finished invoices after a conflict before regrowth may resume.
pub const COOLDOWN_INVOICES: usize = 2 * GROW_EVERY;

#[derive(Debug)]
struct State {
    current: usize,
    ceiling: usize,
    active: usize,
    epoch: u64,
    cooldown: usize,
    clean: usize,
}

#[derive(Debug)]
pub struct PhaseConcurrency {
    state: Mutex<State>,
    freed: Condvar,
}

impl PhaseConcurrency {
    pub fn new(ceiling: usize) -> Self {
        let ceiling = ceiling.max(1);
        Self {
            state: Mutex::new(State {
                current: ceiling,
                ceiling,
                active: 0,
                epoch: 0,
                cooldown: 0,
                clean: 0,
            }),
            freed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How many invoices may run at once right now.
    pub fn limit(&self) -> usize {
        self.lock().current
    }

    /// Blocks until a slot is free and takes it; hand the stamp to `release`.
    pub fn acquire(&self) -> u64 {
        let mut state = self.lock();
        while state.active >= state.current {
            state = self.freed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        state.active += 1;
        state.epoch
    }

    /// Frees the slot and applies the finished invoice's feedback.
    pub fn release(&self, stamp: u64, conflict: bool) {
        let mut state = self.lock();
        state.active = state.active.saturating_sub(1);
        adapt(&mut state, stamp, conflict);
        self.freed.notify_all();
    }
}

fn adapt(state: &mut State, stamp: u64, conflict: bool) {
    if conflict {
        if stamp == state.epoch {
            state.current = (state.current / 2).max(1);
            state.epoch += 1;
        }
        state.cooldown = COOLDOWN_INVOICES;
        state.clean = 0;
        return;
    }
    if state.cooldown > 0 {
        state.cooldown -= 1;
        return;
    }
    if state.current >= state.ceiling {
        return;
    }
    state.clean += 1;
    if state.clean >= GROW_EVERY {
        state.clean = 0;
        state.current += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    fn finish(c: &PhaseConcurrency, conflict: bool) {
        let stamp = c.acquire();
        c.release(stamp, conflict);
    }

    #[test]
    fn it_starts_at_the_ceiling_and_never_exceeds_it() {
        let c = PhaseConcurrency::new(8);
        assert_eq!(c.limit(), 8);
        for _ in 0..200 {
            finish(&c, false);
        }
        assert_eq!(c.limit(), 8);
        assert_eq!(PhaseConcurrency::new(0).limit(), 1);
    }

    #[test]
    fn a_conflict_halves_then_cools_down_then_regrows_one_per_twenty() {
        let c = PhaseConcurrency::new(8);
        finish(&c, true);
        assert_eq!(c.limit(), 4);
        for _ in 0..COOLDOWN_INVOICES {
            finish(&c, false);
        }
        assert_eq!(c.limit(), 4, "no growth during the cooldown");
        for _ in 0..GROW_EVERY - 1 {
            finish(&c, false);
        }
        assert_eq!(c.limit(), 4);
        finish(&c, false);
        assert_eq!(c.limit(), 5);
        for _ in 0..GROW_EVERY * 10 {
            finish(&c, false);
        }
        assert_eq!(c.limit(), 8);
    }

    #[test]
    fn a_conflict_restarts_the_cooldown_and_the_growth_count() {
        let c = PhaseConcurrency::new(8);
        finish(&c, true);
        for _ in 0..COOLDOWN_INVOICES + GROW_EVERY - 1 {
            finish(&c, false);
        }
        finish(&c, true);
        assert_eq!(c.limit(), 2);
        for _ in 0..COOLDOWN_INVOICES + GROW_EVERY - 1 {
            finish(&c, false);
        }
        assert_eq!(c.limit(), 2);
        finish(&c, false);
        assert_eq!(c.limit(), 3);
    }

    #[test]
    fn invoices_admitted_before_a_halving_do_not_halve_again() {
        let c = PhaseConcurrency::new(8);
        let (first, second) = (c.acquire(), c.acquire());
        c.release(first, true);
        assert_eq!(c.limit(), 4);
        c.release(second, true);
        assert_eq!(c.limit(), 4, "same incident: only the cooldown restarts");
    }

    #[test]
    fn the_limit_never_drops_below_one() {
        let c = PhaseConcurrency::new(2);
        for _ in 0..6 {
            finish(&c, true);
        }
        assert_eq!(c.limit(), 1);
    }

    #[test]
    fn acquire_blocks_at_the_limit_until_a_slot_frees() {
        let c = Arc::new(PhaseConcurrency::new(1));
        let stamp = c.acquire();
        let waiter = {
            let c = c.clone();
            std::thread::spawn(move || {
                let s = c.acquire();
                c.release(s, false);
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!waiter.is_finished());
        c.release(stamp, false);
        waiter.join().expect("waiter");
    }
}
