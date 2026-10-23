//! The shared answer cache with request coalescing (Go: `serve_cache.go`).
//!
//! The heavy GET routes (`cargues`, `consulta`, `resumen`) are polled by every
//! open view. [`Cache::get_or_compute`] makes concurrent requests for the same
//! key run one computation and keeps a successful (`200`) answer for the TTL,
//! so a change made by a job shows up at most [`DEFAULT_TTL`] later. Errors are
//! shared only with the callers that were already waiting; they are never kept.
//!
//! Unlike Go there is no ETag, `304` or gzip: the desktop shell strips the
//! validators and a `304` is turned into a `500` by the webview on Windows, so
//! every answer is a plain `200`.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::{errors::ApiError, errors::ApiResult, request::Request};

/// How long a successful answer is shared (Go `responseCacheTTL`).
pub const DEFAULT_TTL: Duration = Duration::from_millis(2500);

/// Distinct keys kept at once; beyond it an answer is computed without being
/// stored, so arbitrary query strings cannot grow the cache.
pub const MAX_KEYS: usize = 32;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Slot {
    /// `None` while the computation runs.
    result: Option<ApiResult>,
    /// Set only for a `200` answer while caching is on.
    expires: Option<Instant>,
}

/// One computation of an answer, in progress or finished.
struct Flight {
    slot: Mutex<Slot>,
    done: Condvar,
}

enum Peek {
    Fresh(ApiResult),
    Running,
    Stale,
}

impl Flight {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            slot: Mutex::new(Slot::default()),
            done: Condvar::new(),
        })
    }

    fn peek(&self, now: Instant) -> Peek {
        let slot = locked(&self.slot);
        match (&slot.result, slot.expires) {
            (None, _) => Peek::Running,
            (Some(result), Some(expires)) if now < expires => Peek::Fresh(result.clone()),
            _ => Peek::Stale,
        }
    }

    fn wait(&self) -> ApiResult {
        let mut slot = locked(&self.slot);
        loop {
            if let Some(result) = &slot.result {
                return result.clone();
            }
            slot = self.done.wait(slot).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn finish(&self, result: ApiResult, expires: Option<Instant>) {
        let mut slot = locked(&self.slot);
        slot.result = Some(result);
        slot.expires = expires;
        drop(slot);
        self.done.notify_all();
    }
}

enum Lookup {
    Hit(ApiResult),
    Wait(Arc<Flight>),
    Lead(Arc<Flight>),
    /// The table is full of fresh answers: compute and answer uncached.
    Bypass,
}

/// Shared TTL cache with coalescing of concurrent computations.
pub struct Cache {
    ttl: Duration,
    flights: Mutex<HashMap<String, Arc<Flight>>>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    /// A cache with [`DEFAULT_TTL`].
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }

    /// A cache with another TTL; a zero TTL only shares computations that are
    /// in flight.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            flights: Mutex::new(HashMap::new()),
        }
    }

    /// The key of a request: method, path and the query with its keys sorted
    /// and percent-encoded (Go `r.Method + " " + path + "?" + Query().Encode()`).
    pub fn key(req: &Request) -> String {
        let mut pairs: Vec<&(String, String)> = req.query.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0)); // stable: values of a key keep their order
        let mut encoder = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in pairs {
            encoder.append_pair(key, value);
        }
        format!("{} {}?{}", req.method, req.path, encoder.finish())
    }

    /// Returns the answer for `key`, running `compute` at most once while the
    /// previous `200` answer is fresh or another caller is computing it. A
    /// panic in `compute` answers `500 internal error` instead of unwinding.
    pub fn get_or_compute(&self, key: &str, compute: impl FnOnce() -> ApiResult) -> ApiResult {
        match self.lookup(key) {
            Lookup::Hit(result) => result,
            Lookup::Wait(flight) => flight.wait(),
            Lookup::Bypass => run(compute),
            Lookup::Lead(flight) => {
                let result = run(compute);
                let cacheable =
                    self.ttl > Duration::ZERO && matches!(&result, Ok(resp) if resp.status == 200);
                let expires = cacheable.then(|| Instant::now() + self.ttl);
                flight.finish(result.clone(), expires);
                if !cacheable {
                    // Only the callers already waiting share a failure.
                    let mut flights = locked(&self.flights);
                    if flights.get(key).is_some_and(|f| Arc::ptr_eq(f, &flight)) {
                        flights.remove(key);
                    }
                }
                result
            }
        }
    }

    /// Number of keys currently held (finished or in flight).
    pub fn len(&self) -> usize {
        locked(&self.flights).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lookup(&self, key: &str) -> Lookup {
        let now = Instant::now();
        let mut flights = locked(&self.flights);
        if let Some(flight) = flights.get(key) {
            match flight.peek(now) {
                Peek::Fresh(result) => return Lookup::Hit(result),
                Peek::Running => return Lookup::Wait(Arc::clone(flight)),
                Peek::Stale => {
                    flights.remove(key);
                }
            }
        }
        if flights.len() >= MAX_KEYS {
            flights.retain(|_, flight| !matches!(flight.peek(now), Peek::Stale));
        }
        if flights.len() >= MAX_KEYS {
            return Lookup::Bypass;
        }
        let flight = Flight::new();
        flights.insert(key.to_owned(), Arc::clone(&flight));
        Lookup::Lead(flight)
    }
}

fn run(compute: impl FnOnce() -> ApiResult) -> ApiResult {
    catch_unwind(AssertUnwindSafe(compute)).unwrap_or_else(|_| {
        tracing::error!("api: handler panicked while computing a shared answer");
        Err(ApiError::internal("internal error"))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use super::*;
    use crate::api::request::Response;

    fn ok(body: &str) -> ApiResult {
        Ok(Response::json(&serde_json::json!({ "v": body })))
    }

    #[test]
    fn concurrent_callers_share_one_computation() {
        let cache = Arc::new(Cache::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(6));
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let (cache, runs, barrier) = (cache.clone(), runs.clone(), barrier.clone());
                thread::spawn(move || {
                    barrier.wait();
                    cache.get_or_compute("k", || {
                        runs.fetch_add(1, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(250));
                        ok("shared")
                    })
                })
            })
            .collect();
        for handle in handles {
            let resp = handle.join().expect("thread").expect("200");
            assert_eq!(resp.status, 200);
            assert_eq!(resp.body, b"{\"v\":\"shared\"}\n");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_200_is_kept_for_the_ttl_and_then_recomputed() {
        let cache = Cache::with_ttl(Duration::from_millis(80));
        let runs = AtomicUsize::new(0);
        let call = || {
            cache.get_or_compute("k", || {
                runs.fetch_add(1, Ordering::SeqCst);
                ok("x")
            })
        };
        call().expect("first");
        call().expect("cached");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        thread::sleep(Duration::from_millis(160));
        call().expect("recomputed");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn errors_and_non_200_answers_are_never_kept() {
        let cache = Cache::new();
        let runs = AtomicUsize::new(0);
        let failing = cache.get_or_compute("e", || {
            runs.fetch_add(1, Ordering::SeqCst);
            Err(ApiError::new(503, "caído"))
        });
        assert_eq!(failing.expect_err("error").status, 503);
        let odd = cache.get_or_compute("e", || {
            runs.fetch_add(1, Ordering::SeqCst);
            Ok(Response::new(204))
        });
        assert_eq!(odd.expect("204").status, 204);
        cache
            .get_or_compute("e", || {
                runs.fetch_add(1, Ordering::SeqCst);
                ok("now")
            })
            .expect("ok");
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        assert_eq!(cache.len(), 1, "only the 200 stays");
    }

    #[test]
    fn a_zero_ttl_shares_nothing_after_the_computation() {
        let cache = Cache::with_ttl(Duration::ZERO);
        let runs = AtomicUsize::new(0);
        for _ in 0..3 {
            cache
                .get_or_compute("k", || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    ok("x")
                })
                .expect("ok");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        assert!(cache.is_empty());
    }

    #[test]
    fn at_most_32_keys_are_kept_and_the_rest_is_answered_uncached() {
        let cache = Cache::new();
        for i in 0..MAX_KEYS {
            cache
                .get_or_compute(&format!("k{i}"), || ok("x"))
                .expect("ok");
        }
        assert_eq!(cache.len(), MAX_KEYS);
        let runs = AtomicUsize::new(0);
        for _ in 0..2 {
            cache
                .get_or_compute("one-too-many", || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    ok("y")
                })
                .expect("ok");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 2, "never stored");
        assert_eq!(cache.len(), MAX_KEYS);
        // The kept ones still answer from the cache.
        cache
            .get_or_compute("k0", || Err(ApiError::internal("recomputed")))
            .expect("still cached");
    }

    #[test]
    fn stale_keys_make_room_for_new_ones() {
        let cache = Cache::with_ttl(Duration::from_millis(40));
        for i in 0..MAX_KEYS {
            cache
                .get_or_compute(&format!("k{i}"), || ok("x"))
                .expect("ok");
        }
        thread::sleep(Duration::from_millis(100));
        let runs = AtomicUsize::new(0);
        for _ in 0..2 {
            cache
                .get_or_compute("fresh", || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    ok("y")
                })
                .expect("ok");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_panicking_computation_answers_500_and_does_not_poison_the_key() {
        let cache = Cache::new();
        let err = cache
            .get_or_compute("p", || panic!("boom"))
            .expect_err("500");
        assert_eq!((err.status, err.message.as_str()), (500, "internal error"));
        cache.get_or_compute("p", || ok("fine")).expect("recovers");
    }

    #[test]
    fn the_key_sorts_query_keys_and_keeps_the_order_of_repeated_values() {
        let a = Request::new("GET", "/api/cargues")
            .with_query("b", "2")
            .with_query("a", "x y")
            .with_query("b", "1");
        let b = Request::new("GET", "/api/cargues")
            .with_query("a", "x y")
            .with_query("b", "2")
            .with_query("b", "1");
        assert_eq!(Cache::key(&a), "GET /api/cargues?a=x+y&b=2&b=1");
        assert_eq!(Cache::key(&a), Cache::key(&b));
        assert_eq!(
            Cache::key(&Request::new("GET", "/api/resumen")),
            "GET /api/resumen?"
        );
    }
}
