//! Wall-clock timing of the command's steps, measured with the monotonic
//! [`Instant`], and the timestamp-based ids shared by loads and Nota Técnica
//! versions.

use std::time::{Duration, Instant};

use chrono::Local;
use uuid::Uuid;

/// Step durations in the order they were recorded.
#[derive(Debug, Default)]
pub struct StepTimings {
    steps: Vec<(&'static str, Duration)>,
}

impl StepTimings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an already measured duration.
    pub fn record(&mut self, label: &'static str, elapsed: Duration) {
        tracing::debug!(
            step = label,
            elapsed_ms = elapsed.as_millis() as u64,
            "step finished"
        );
        self.steps.push((label, elapsed));
    }

    /// Runs `step`, records how long it took under `label` and returns its result.
    pub fn time<T>(&mut self, label: &'static str, step: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = step();
        self.record(label, started.elapsed());
        result
    }

    pub fn steps(&self) -> &[(&'static str, Duration)] {
        &self.steps
    }
}

/// Runs `step` and returns its result together with its duration.
pub fn measure<T>(step: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let result = step();
    (result, started.elapsed())
}

/// Builds `YYYYMMDD_HHMMSS_<8 hex chars of a v4 UUID>`, using local time.
///
/// The random suffix keeps ids unique even within the same second.
pub fn new_timestamped_id() -> String {
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let suffix = &Uuid::new_v4().simple().to_string()[..8];
    format!("{timestamp}_{suffix}")
}

/// Lines of the "Tiempos:" section, preceded by a blank line.
pub fn timings_section(timings: &StepTimings) -> Vec<String> {
    let mut lines = vec![String::new(), "Tiempos:".to_owned()];
    lines.extend(
        timings
            .steps()
            .iter()
            .map(|(label, elapsed)| format!("  {label}: {}", format_duration(*elapsed))),
    );
    lines
}

/// Human-readable duration, truncated to whole milliseconds:
/// `< 1 ms`, `NN ms`, `N.NNN s` (from 1 s) or `Nm NN.NNNs` (from 60 s).
pub fn format_duration(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    match millis {
        0 => "< 1 ms".to_owned(),
        1..1_000 => format!("{millis} ms"),
        1_000..60_000 => format!("{}.{:03} s", millis / 1_000, millis % 1_000),
        _ => {
            let rest = millis % 60_000;
            format!(
                "{}m {:02}.{:03}s",
                millis / 60_000,
                rest / 1_000,
                rest % 1_000
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn formats_sub_millisecond_as_less_than_one() {
        assert_eq!(format_duration(Duration::ZERO), "< 1 ms");
        assert_eq!(format_duration(Duration::from_micros(999)), "< 1 ms");
    }

    #[test]
    fn formats_milliseconds_below_one_second() {
        assert_eq!(format_duration(ms(1)), "1 ms");
        assert_eq!(format_duration(ms(87)), "87 ms");
        assert_eq!(format_duration(ms(999)), "999 ms");
    }

    #[test]
    fn formats_seconds_with_three_decimals() {
        assert_eq!(format_duration(ms(1_000)), "1.000 s");
        assert_eq!(format_duration(ms(1_234)), "1.234 s");
        assert_eq!(format_duration(ms(59_999)), "59.999 s");
    }

    #[test]
    fn formats_minutes_and_padded_seconds() {
        assert_eq!(format_duration(ms(60_000)), "1m 00.000s");
        assert_eq!(format_duration(ms(125_123)), "2m 05.123s");
    }

    #[test]
    fn record_keeps_insertion_order_and_labels() {
        let mut timings = StepTimings::new();
        timings.record("b", ms(2));
        timings.record("a", ms(1));

        assert_eq!(timings.steps(), &[("b", ms(2)), ("a", ms(1))]);
    }

    #[test]
    fn time_returns_the_step_result_and_records_it() {
        let mut timings = StepTimings::new();

        let value = timings.time("step", || 42);

        assert_eq!(value, 42);
        assert_eq!(timings.steps().len(), 1);
        assert_eq!(timings.steps()[0].0, "step");
    }

    #[test]
    fn timestamped_id_has_timestamp_and_8_hex_suffix() {
        let id = new_timestamped_id();
        let parts: Vec<&str> = id.split('_').collect();

        assert_eq!(parts.len(), 3, "unexpected id: {id}");
        assert_eq!(parts[0].len(), 8);
        assert!(parts[0].bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(parts[1].len(), 6);
        assert!(parts[1].bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(parts[2].len(), 8);
        assert!(parts[2].bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn timestamped_ids_are_unique() {
        assert_ne!(new_timestamped_id(), new_timestamped_id());
    }

    #[test]
    fn timings_section_lists_labeled_steps_in_order() {
        let mut timings = StepTimings::new();
        timings.record("Validación", ms(2));
        timings.record("Copia", Duration::ZERO);
        timings.record("Total", ms(1_342));

        assert_eq!(
            timings_section(&timings),
            vec![
                "",
                "Tiempos:",
                "  Validación: 2 ms",
                "  Copia: < 1 ms",
                "  Total: 1.342 s",
            ]
        );
    }

    #[test]
    fn measure_returns_the_step_result() {
        let (value, _elapsed) = measure(|| "done");
        assert_eq!(value, "done");
    }
}
