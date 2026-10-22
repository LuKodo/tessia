//! AIMD adaptive concurrency limiter (port of Go `httpx.AdaptiveLimiter`, itself
//! the legacy Java `ReguladorDescargaAdaptativo`).
//!
//! After every `window` results: an error rate above 15% cuts the concurrency
//! to 65% (floor, not below `min`) and doubles the pacing pause (at least
//! +100 ms, capped at `pause_max`); two consecutive windows under 3% errors add
//! 3 slots (up to `max`) and shorten the pause by 50 ms (not below
//! `pause_min`); anything in between resets the stability counter.
//!
//! ```
//! use tessia::http::{AdaptiveLimiter, LimiterOptions};
//! use std::time::Duration;
//!
//! let limiter = AdaptiveLimiter::new(LimiterOptions {
//!     name: "demo".into(), min: 1, initial: 2, max: 4, window: 20,
//!     pause_min: Duration::ZERO, pause_max: Duration::from_millis(100),
//! });
//! let permit = limiter.acquire();
//! // ... one HTTP exchange ...
//! permit.finish(true); // healthy answer
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Parameters of one limiter.
#[derive(Debug, Clone)]
pub struct LimiterOptions {
    pub name: String,
    /// Floor of the concurrency (>= 1).
    pub min: usize,
    /// Starting concurrency, clamped to `[min, max]`.
    pub initial: usize,
    pub max: usize,
    /// Results per adjustment (>= 20).
    pub window: usize,
    pub pause_min: Duration,
    pub pause_max: Duration,
}

/// The numbers of one external system, as constants.
#[derive(Debug, Clone, Copy)]
pub struct LimiterSpec {
    pub name: &'static str,
    pub min: usize,
    pub initial: usize,
    pub max: usize,
    pub window: usize,
    pub pause_min_ms: u64,
    pub pause_max_ms: u64,
}

impl LimiterSpec {
    pub fn options(&self) -> LimiterOptions {
        LimiterOptions {
            name: self.name.to_owned(),
            min: self.min,
            initial: self.initial,
            max: self.max,
            window: self.window,
            pause_min: Duration::from_millis(self.pause_min_ms),
            pause_max: Duration::from_millis(self.pause_max_ms),
        }
    }
}

/// The AIMD parameters Go used per external system (min / initial / max
/// threads, result window, pacing pause range). Reuse them; do not invent new
/// numbers.
pub mod specs {
    use super::LimiterSpec;

    /// The Factory (HKA): 4 / 20 / 30, window 100, pause 40..900 ms.
    pub const THEFACTORY: LimiterSpec = LimiterSpec {
        name: "thefactory",
        min: 4,
        initial: 20,
        max: 30,
        window: 100,
        pause_min_ms: 40,
        pause_max_ms: 900,
    };
    /// SISPRO (the legacy MUTUALSER numbers the Java CUV, FE-zip and JSON
    /// steps used): 8 / 20 / 40, window 80, pause 25..900 ms.
    pub const SISPRO: LimiterSpec = LimiterSpec {
        name: "sispro",
        min: 8,
        initial: 20,
        max: 40,
        window: 80,
        pause_min_ms: 25,
        pause_max_ms: 900,
    };
    /// CPScan: 6 / 24 / 35, window 100, pause 25..700 ms.
    pub const CPSCAN: LimiterSpec = LimiterSpec {
        name: "cpscan",
        min: 6,
        initial: 24,
        max: 35,
        window: 100,
        pause_min_ms: 25,
        pause_max_ms: 700,
    };
    /// Mutual Ser (ZonaSer): 8 / 20 / 40, window 80, pause 25..900 ms.
    pub const MUTUALSER: LimiterSpec = LimiterSpec {
        name: "mutualser",
        min: 8,
        initial: 20,
        max: 40,
        window: 80,
        pause_min_ms: 25,
        pause_max_ms: 900,
    };
    /// Derechos validator: 4 / 16 / `max`, window 80, pause 25..900 ms, where
    /// `max` is the validator concurrency (Go default 30).
    pub const DERECHOS_BASE: LimiterSpec = LimiterSpec {
        name: "derechos",
        min: 4,
        initial: 16,
        max: 30,
        window: 80,
        pause_min_ms: 25,
        pause_max_ms: 900,
    };

    /// Derechos with a custom maximum concurrency.
    pub fn derechos(max: usize) -> LimiterSpec {
        LimiterSpec {
            max: max.max(DERECHOS_BASE.min),
            ..DERECHOS_BASE
        }
    }
}

#[derive(Debug)]
struct State {
    current: usize,
    active: usize,
    events: usize,
    errors: usize,
    stable_windows: u32,
    pause: Duration,
    last: Option<Instant>,
}

/// Process-wide AIMD limiter; share it between threads (`Arc` or a borrow).
#[derive(Debug)]
pub struct AdaptiveLimiter {
    name: String,
    min: usize,
    max: usize,
    window: usize,
    pause_min: Duration,
    pause_max: Duration,
    state: Mutex<State>,
    freed: Condvar,
}

/// A held slot. Give it back with [`Permit::finish`]; a permit dropped
/// without an outcome counts as a failure (the conservative reading).
#[must_use = "a dropped permit counts as a failed exchange"]
pub struct Permit<'a> {
    limiter: &'a AdaptiveLimiter,
    done: bool,
}

impl Permit<'_> {
    /// Releases the slot, feeding the result into the AIMD window. `healthy`
    /// is false for transport errors, timeouts, 429 and 5xx.
    pub fn finish(mut self, healthy: bool) {
        self.done = true;
        self.limiter.release(healthy);
    }

    /// Releases the slot from an HTTP status (see [`response_healthy`]).
    pub fn finish_status(self, status: u16) {
        self.finish(response_healthy(status));
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.limiter.release(false);
        }
    }
}

/// Whether an HTTP status counts as a healthy answer for the limiter: anything
/// but 429 and 5xx (client errors say nothing about the server's capacity).
pub fn response_healthy(status: u16) -> bool {
    status < 500 && status != 429
}

impl AdaptiveLimiter {
    pub fn new(options: LimiterOptions) -> Self {
        let min = options.min.max(1);
        let max = options.max.max(min);
        let pause_min = options.pause_min;
        Self {
            name: options.name,
            min,
            max,
            window: options.window.max(20),
            pause_min,
            pause_max: options.pause_max.max(pause_min),
            state: Mutex::new(State {
                current: options.initial.clamp(min, max),
                active: 0,
                events: 0,
                errors: 0,
                stable_windows: 0,
                pause: pause_min,
                last: None,
            }),
            freed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Blocks until a slot is free and the pacing pause since the previous
    /// acquisition has elapsed.
    pub fn acquire(&self) -> Permit<'_> {
        let mut state = self.lock();
        loop {
            let now = Instant::now();
            let wait = state.last.map_or(Duration::ZERO, |last| {
                (last + state.pause).saturating_duration_since(now)
            });
            if state.active < state.current && wait.is_zero() {
                state.active += 1;
                state.last = Some(now);
                return Permit {
                    limiter: self,
                    done: false,
                };
            }
            // Safety re-check every 250 ms, like the Java version.
            let timeout = if wait.is_zero() {
                Duration::from_millis(250)
            } else {
                wait
            };
            state = self
                .freed
                .wait_timeout(state, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Runs one exchange under a slot: `exchange` must perform the request
    /// AND read the response body and nothing else (no retry sleeps, parsing
    /// or file writes), returning the HTTP status when there was one. A
    /// transport error (`Err`) counts as unhealthy.
    pub fn exchange<T, E>(&self, exchange: impl FnOnce() -> Result<(u16, T), E>) -> Result<T, E> {
        let permit = self.acquire();
        match exchange() {
            Ok((status, value)) => {
                permit.finish_status(status);
                Ok(value)
            }
            Err(err) => {
                permit.finish(false);
                Err(err)
            }
        }
    }

    fn release(&self, healthy: bool) {
        let mut state = self.lock();
        state.active = state.active.saturating_sub(1);
        state.events += 1;
        if !healthy {
            state.errors += 1;
        }
        if state.events >= self.window {
            self.adjust(&mut state);
        }
        drop(state);
        self.freed.notify_all();
    }

    fn adjust(&self, state: &mut State) {
        let rate = state.errors as f64 / state.events as f64;
        let (before, pause_before) = (state.current, state.pause);
        if rate > 0.15 {
            state.current = self.min.max((state.current as f64 * 0.65).floor() as usize);
            state.pause = self
                .pause_max
                .min((state.pause + Duration::from_millis(100)).max(state.pause * 2));
            state.stable_windows = 0;
        } else if rate < 0.03 {
            state.stable_windows += 1;
            if state.stable_windows >= 2 {
                state.current = self.max.min(state.current + 3);
                state.pause = self
                    .pause_min
                    .max(state.pause.saturating_sub(Duration::from_millis(50)));
                state.stable_windows = 0;
            }
        } else {
            state.stable_windows = 0;
        }
        if before != state.current || pause_before != state.pause {
            tracing::info!(
                limiter = %self.name,
                concurrency = state.current,
                pause_ms = state.pause.as_millis() as u64,
                errors = state.errors,
                events = state.events,
                "adaptive limiter adjusted"
            );
        }
        state.events = 0;
        state.errors = 0;
    }

    /// Current concurrency limit.
    pub fn current(&self) -> usize {
        self.lock().current
    }

    /// Slots held right now.
    pub fn active(&self) -> usize {
        self.lock().active
    }

    /// Current pacing pause between acquisitions.
    pub fn pause(&self) -> Duration {
        self.lock().pause
    }

    pub fn max(&self) -> usize {
        self.max
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The process-wide limiters by system name (`thefactory`, `sispro`, `cpscan`,
/// `mutualser`, `derechos`), built on first use. `proceso run` calls the phases
/// once per invoice on many threads, so a limiter owned by a client would pace
/// nothing: every client of a system shares the one from here.
#[derive(Debug, Default)]
pub struct Limiters {
    by_name: Mutex<HashMap<String, Arc<AdaptiveLimiter>>>,
}

impl Limiters {
    pub fn new() -> Self {
        Self::default()
    }

    /// The shared limiter of `system`; `None` for an unknown system
    /// (unlimited).
    pub fn get(&self, system: &str) -> Option<Arc<AdaptiveLimiter>> {
        let spec = match system {
            "thefactory" => specs::THEFACTORY,
            "sispro" => specs::SISPRO,
            "cpscan" => specs::CPSCAN,
            "mutualser" => specs::MUTUALSER,
            "derechos" => specs::DERECHOS_BASE,
            _ => return None,
        };
        let mut map = self
            .by_name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(
            map.entry(system.to_owned())
                .or_insert_with(|| Arc::new(AdaptiveLimiter::new(spec.options())))
                .clone(),
        )
    }

    /// Registers a custom limiter under `system` (e.g. derechos with a custom
    /// maximum); replaces any existing one.
    pub fn insert(&self, system: &str, limiter: AdaptiveLimiter) -> Arc<AdaptiveLimiter> {
        let limiter = Arc::new(limiter);
        let mut map = self
            .by_name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.insert(system.to_owned(), limiter.clone());
        limiter
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use super::*;

    fn limiter(initial: usize, max: usize) -> AdaptiveLimiter {
        AdaptiveLimiter::new(LimiterOptions {
            name: "t".into(),
            min: 2,
            initial,
            max,
            window: 20,
            pause_min: Duration::ZERO,
            pause_max: Duration::from_millis(400),
        })
    }

    fn feed(limiter: &AdaptiveLimiter, ok: usize, failed: usize) {
        // Releasing directly keeps the tests fast: acquiring would also wait
        // out the pacing pause the failures themselves lengthen.
        for i in 0..ok + failed {
            limiter.release(i >= failed);
        }
    }

    #[test]
    fn heavy_errors_shrink_concurrency_and_lengthen_the_pause() {
        let l = limiter(10, 20);
        feed(&l, 10, 10); // 50% errors over a window of 20
        assert_eq!(l.current(), 6); // floor(10 * 0.65)
        assert_eq!(l.pause(), Duration::from_millis(100));
        feed(&l, 0, 20);
        assert_eq!(l.current(), 3);
        assert_eq!(l.pause(), Duration::from_millis(200));
        for _ in 0..4 {
            feed(&l, 0, 20);
        }
        assert_eq!(l.current(), 2, "never below min");
        assert_eq!(
            l.pause(),
            Duration::from_millis(400),
            "never above pause_max"
        );
    }

    #[test]
    fn two_clean_windows_grow_concurrency_and_shorten_the_pause() {
        let l = limiter(4, 8);
        feed(&l, 0, 20);
        assert_eq!(l.current(), 2);
        feed(&l, 20, 0);
        assert_eq!(l.current(), 2, "one clean window is not enough");
        feed(&l, 20, 0);
        assert_eq!(l.current(), 5);
        assert_eq!(l.pause(), Duration::from_millis(50));
        for _ in 0..8 {
            feed(&l, 20, 0);
        }
        assert_eq!(l.current(), 8, "never above max");
        assert_eq!(l.pause(), Duration::ZERO);
    }

    #[test]
    fn middling_error_rates_change_nothing() {
        let l = limiter(10, 20);
        for _ in 0..3 {
            feed(&l, 18, 2); // 10%
        }
        assert_eq!(l.current(), 10);
    }

    #[test]
    fn a_dropped_permit_counts_as_a_failure() {
        let l = limiter(10, 20);
        for _ in 0..20 {
            drop(l.acquire());
        }
        assert_eq!(l.current(), 6);
    }

    #[test]
    fn concurrency_never_exceeds_the_limit_across_threads() {
        let l = Arc::new(limiter(3, 3));
        let (running, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (l, running, peak) = (l.clone(), running.clone(), peak.clone());
                thread::spawn(move || {
                    for _ in 0..5 {
                        let permit = l.acquire();
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(2));
                        running.fetch_sub(1, Ordering::SeqCst);
                        permit.finish(true);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("worker");
        }
        assert!(peak.load(Ordering::SeqCst) <= 3);
        assert_eq!(l.active(), 0);
    }

    #[test]
    fn pacing_pause_spaces_acquisitions() {
        let l = AdaptiveLimiter::new(LimiterOptions {
            name: "p".into(),
            min: 1,
            initial: 5,
            max: 5,
            window: 20,
            pause_min: Duration::from_millis(40),
            pause_max: Duration::from_millis(40),
        });
        let started = Instant::now();
        l.acquire().finish(true);
        l.acquire().finish(true);
        assert!(started.elapsed() >= Duration::from_millis(40));
    }

    #[test]
    fn exchange_classifies_by_status_and_errors() {
        let l = limiter(10, 20);
        for _ in 0..20 {
            let _ = l.exchange(|| Ok::<_, ()>((503, ())));
        }
        assert_eq!(l.current(), 6);
        assert_eq!(l.exchange(|| Err::<(u16, ()), _>("boom")), Err("boom"));
        assert_eq!(l.active(), 0);
    }

    #[test]
    fn registry_shares_limiters_by_system_and_knows_the_go_numbers() {
        let limiters = Limiters::new();
        let a = limiters.get("thefactory").expect("known");
        let b = limiters.get("thefactory").expect("known");
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!((a.current(), a.max()), (20, 30));
        assert_eq!(limiters.get("sispro").map(|l| l.max()), Some(40));
        assert_eq!(limiters.get("cpscan").map(|l| l.current()), Some(24));
        assert!(limiters.get("unknown").is_none());
        assert_eq!(specs::derechos(60).max, 60);
    }
}
