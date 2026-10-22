//! Bounded parallel map over OS threads (the program is synchronous: threads,
//! blocking HTTP, no async runtime).

use crossbeam_channel::unbounded;

/// Applies `work` to every item using at most `workers` threads and returns the
/// results in the order of `items`. `workers` is clamped to `1..=items.len()`.
/// A panic in `work` propagates to the caller.
///
/// ```
/// let squares = tessia::util::par::map(vec![1, 2, 3, 4], 3, |n| n * n);
/// assert_eq!(squares, [1, 4, 9, 16]);
/// ```
pub fn map<T, R, F>(items: Vec<T>, workers: usize, work: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync,
{
    let total = items.len();
    let workers = workers.clamp(1, total.max(1));
    let (job_tx, job_rx) = unbounded::<(usize, T)>();
    let (result_tx, result_rx) = unbounded::<(usize, R)>();
    for job in items.into_iter().enumerate() {
        // The receiver is alive: it is held below until the scope ends.
        let _ = job_tx.send(job);
    }
    drop(job_tx);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (job_rx, result_tx, work) = (job_rx.clone(), result_tx.clone(), &work);
            scope.spawn(move || {
                for (index, item) in job_rx {
                    let _ = result_tx.send((index, work(item)));
                }
            });
        }
        drop(result_tx);
    });
    let mut results: Vec<(usize, R)> = result_rx.into_iter().collect();
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn results_keep_the_input_order() {
        let out = map((0..100).collect(), 8, |n: u32| {
            if n.is_multiple_of(7) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            n * 2
        });
        assert_eq!(out, (0..100).map(|n| n * 2).collect::<Vec<_>>());
    }

    #[test]
    fn never_runs_more_than_the_requested_workers() {
        let (running, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        map(vec![(); 30], 3, |()| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(3));
            running.fetch_sub(1, Ordering::SeqCst);
        });
        assert!(peak.load(Ordering::SeqCst) <= 3);
    }

    #[test]
    fn empty_input_and_zero_workers_are_fine() {
        assert!(map(Vec::<u8>::new(), 0, |n| n).is_empty());
        assert_eq!(map(vec![1], 0, |n| n + 1), [2]);
    }
}
