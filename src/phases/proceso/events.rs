//! The progress event stream of `proceso run` (Go `procesoEvent` and its
//! three renderers). Stdout carries only this stream; the phases' own
//! chatter goes to stderr.
//!
//! Event types: `convenio_start`, `convenio`, `convenio_more`, `run_start`,
//! `invoice_enter`, `invoice_step`, `invoice_replace`, `replace_stopped`,
//! `radicacion`, `run_end`. `--json` prints each event as one compact JSON
//! line with the Go field names; the text renderer prints one line per step
//! and a summary; `--panel` repaints a table on a terminal and prints plain
//! lines when stdout is not one. The text lines are the Go ones on purpose
//! (the stream is a documented contract).
//!
//! New with multi-machine runs: an `invoice_step` with status `SKIPPED` and
//! reason `EN_PROCESO_OTRA_MAQUINA` means another machine owns that invoice
//! in that phase; `run_end` then carries `skipped` (omitted when 0).

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Local, SecondsFormat};
use serde::Serialize;

/// Output mode of the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Text,
    Json,
    Panel,
}

/// One event; fields default to their zero value and are omitted from the
/// JSON like Go's `omitempty`.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub invoice: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub phase: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub step: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(rename = "durationMs", skip_serializing_if = "is_zero_u64")]
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub batch: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invoices: Vec<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub total: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub done: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub manual: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub excluded: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub failed: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub skipped: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub replaced: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub evaluated: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub limit: usize,
    #[serde(rename = "phaseMs", skip_serializing_if = "BTreeMap::is_empty")]
    pub phase_ms: BTreeMap<String, u64>,
    pub time: String,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

impl Event {
    /// An event of `kind` stamped with the current local time.
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            time: now_stamp(),
            ..Self::default()
        }
    }

    pub fn invoice(mut self, invoice: &str) -> Self {
        self.invoice = invoice.to_owned();
        self
    }

    pub fn phase(mut self, phase: &str, step: &str) -> Self {
        self.phase = phase.to_owned();
        self.step = step.to_owned();
        self
    }

    pub fn step(mut self, step: &str) -> Self {
        self.step = step.to_owned();
        self
    }

    pub fn status(mut self, status: &str) -> Self {
        self.status = status.to_owned();
        self
    }

    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = reason.to_owned();
        self
    }

    pub fn error(mut self, error: &str) -> Self {
        self.error = error.to_owned();
        self
    }

    pub fn millis(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }
}

/// RFC 3339 local timestamp with sub-second digits (Go `RFC3339Nano`).
pub fn now_stamp() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::AutoSi, false)
}

/// The single-character indicator of a status.
pub fn glyph(status: &str) -> &'static str {
    match status {
        "running" => "▸",
        "OK" => "✓",
        "MANUAL_REVIEW" => "⚠",
        "EXCLUDED" => "⊘",
        "FAILED" => "✗",
        _ => "·",
    }
}

/// `420ms` or `1.5s`.
pub fn format_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// Console pointer to the manual review report.
pub fn manual_review_hint(n: usize) -> String {
    format!(
        "manual review: {n} invoice(s) to review; open \"Informe de revisión manual\" in the dashboard"
    )
}

#[derive(Debug, Clone, Copy, Default)]
struct Count {
    inflight: usize,
    done: usize,
    manual: usize,
    excluded: usize,
    failed: usize,
    skipped: usize,
}

const PANEL_TAIL: usize = 6;
const PANEL_REPAINT: Duration = Duration::from_millis(200);

struct Inner {
    out: Box<dyn Write + Send>,
    /// Phases in order of first appearance with their derived counts.
    counts: Vec<(String, Count)>,
    started: Option<Instant>,
    total: usize,
    tail: Vec<String>,
    filed: usize,
    drawn: usize,
    last_paint: Option<Instant>,
}

type Tap = Box<dyn Fn(&Event) + Send + Sync>;

/// The sink every part of the run writes to; safe for concurrent use.
pub struct Events {
    mode: Mode,
    tty: bool,
    inner: Mutex<Inner>,
    tap: Option<Tap>,
}

impl Events {
    /// The stream on the real stdout; a terminal repaints the panel.
    pub fn stdout(mode: Mode) -> Self {
        let tty = std::io::stdout().is_terminal();
        Self::new(mode, Box::new(std::io::stdout()), tty)
    }

    pub fn new(mode: Mode, out: Box<dyn Write + Send>, tty: bool) -> Self {
        Self {
            mode,
            tty,
            inner: Mutex::new(Inner {
                out,
                counts: Vec::new(),
                started: None,
                total: 0,
                tail: Vec::new(),
                filed: 0,
                drawn: 0,
                last_paint: None,
            }),
            tap: None,
        }
    }

    /// Also hands every event to `tap` (tests, dashboards).
    #[must_use]
    pub fn with_tap(mut self, tap: impl Fn(&Event) + Send + Sync + 'static) -> Self {
        self.tap = Some(Box::new(tap));
        self
    }

    pub fn emit(&self, event: Event) {
        if let Some(tap) = &self.tap {
            tap(&event);
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match self.mode {
            Mode::Json => write_json(&mut inner, &event),
            Mode::Text => text(&mut inner, &event),
            Mode::Panel => panel(&mut inner, &event, self.tty),
        }
        let _ = inner.out.flush();
    }
}

fn write_json(inner: &mut Inner, event: &Event) {
    if let Ok(line) = serde_json::to_string(event) {
        let _ = writeln!(inner.out, "{line}");
    }
}

fn count_of<'a>(inner: &'a mut Inner, phase: &str) -> &'a mut Count {
    let at = match inner.counts.iter().position(|(name, _)| name == phase) {
        Some(at) => at,
        None => {
            inner.counts.push((phase.to_owned(), Count::default()));
            inner.counts.len() - 1
        }
    };
    &mut inner.counts[at].1
}

fn leave(count: &mut Count, status: &str) {
    count.inflight = count.inflight.saturating_sub(1);
    match status {
        "OK" => count.done += 1,
        "EXCLUDED" => count.excluded += 1,
        "MANUAL_REVIEW" => count.manual += 1,
        "FAILED" => count.failed += 1,
        "SKIPPED" => count.skipped += 1,
        _ => {}
    }
}

fn detail(event: &Event) -> String {
    if !event.reason.is_empty() {
        format!("  {}", event.reason)
    } else if !event.error.is_empty() {
        format!("  {}", event.error)
    } else {
        String::new()
    }
}

fn text(inner: &mut Inner, e: &Event) {
    let line = match e.kind.as_str() {
        "convenio_start" => format!("{:<12} {:<18} ▸ preparando…", "convenio", e.step),
        "run_start" => format!("proceso: {} invoice(s) entering the pipeline", e.total),
        "convenio" => format!(
            "{:<12} {:<18} {} {}{}",
            "convenio",
            e.step,
            glyph(&e.status),
            format_ms(e.duration_ms),
            detail(e)
        ),
        "invoice_enter" => {
            count_of(inner, &e.phase).inflight += 1;
            return;
        }
        "invoice_step" => {
            leave(count_of(inner, &e.phase), &e.status);
            format!(
                "{:<12} {:<12} {} {:<14} {}{}",
                e.invoice,
                e.phase,
                glyph(&e.status),
                e.status,
                format_ms(e.duration_ms),
                detail(e)
            )
        }
        "radicacion" => format!(
            "{:<12} {:<12} {} {:<14} {}  ({} invoice(s)){}",
            "-",
            "radicacion",
            glyph(&e.status),
            e.status,
            format_ms(e.duration_ms),
            e.batch,
            detail(e)
        ),
        "convenio_more" => format!(
            "convenio  +{} prepared, {} excluded, {} manual (evaluated {})",
            e.done, e.excluded, e.manual, e.evaluated
        ),
        "invoice_replace" => format!("{:<12} {}  (after {})", e.invoice, e.reason, e.phase),
        "replace_stopped" => format!("replacement stopped: {}", e.reason),
        "run_end" => {
            run_end_text(inner, e);
            return;
        }
        _ => return,
    };
    let _ = writeln!(inner.out, "{line}");
}

fn run_end_text(inner: &mut Inner, e: &Event) {
    let mut extra = String::new();
    if e.replaced > 0 {
        extra.push_str(&format!(" replaced={}", e.replaced));
    }
    if e.skipped > 0 {
        extra.push_str(&format!(" skipped={}", e.skipped));
    }
    let _ = writeln!(
        inner.out,
        "\nproceso: {} (done={} manual={} excluded={} failed={}{extra})",
        e.status, e.done, e.manual, e.excluded, e.failed
    );
    let rows: Vec<String> = inner
        .counts
        .iter()
        .map(|(phase, c)| {
            format!(
                "  {phase:<12} done={} manual={} excluded={} failed={}",
                c.done, c.manual, c.excluded, c.failed
            )
        })
        .collect();
    for row in rows {
        let _ = writeln!(inner.out, "{row}");
    }
    if e.manual > 0 {
        let _ = writeln!(inner.out, "{}", manual_review_hint(e.manual));
    }
}

fn panel(inner: &mut Inner, e: &Event, tty: bool) {
    if inner.started.is_none() && matches!(e.kind.as_str(), "convenio_start" | "run_start") {
        inner.started = Some(Instant::now());
    }
    let line = match e.kind.as_str() {
        "convenio_start" => {
            count_of(inner, "convenio");
            Some("convenio preparar ▸ preparando…".to_owned())
        }
        "run_start" => {
            inner.total = e.total;
            if !tty {
                let _ = writeln!(inner.out, "proceso: {} invoice(s)", e.total);
            }
            None
        }
        "convenio" => {
            let count = count_of(inner, "convenio");
            if e.status == "OK" {
                count.done += 1;
            } else {
                count.failed += 1;
            }
            Some(format!(
                "convenio {} {}",
                glyph(&e.status),
                format_ms(e.duration_ms)
            ))
        }
        "invoice_enter" => {
            count_of(inner, &e.phase).inflight += 1;
            None
        }
        "invoice_step" => {
            leave(count_of(inner, &e.phase), &e.status);
            Some(format!(
                "{} {} {} {}",
                e.invoice,
                e.phase,
                glyph(&e.status),
                format_ms(e.duration_ms)
            ))
        }
        "convenio_more" => Some(format!(
            "convenio +{} prepared, {} excluded, {} manual",
            e.done, e.excluded, e.manual
        )),
        "invoice_replace" => Some(format!("{} {}", e.invoice, e.reason)),
        "replace_stopped" => Some(format!("replacement stopped: {}", e.reason)),
        "radicacion" => {
            if e.status == "OK" {
                inner.filed += e.batch;
            }
            Some(format!(
                "radicacion {} {} invoice(s) {}",
                glyph(&e.status),
                e.batch,
                format_ms(e.duration_ms)
            ))
        }
        "run_end" => {
            if !tty {
                let elapsed = inner.started.map_or(0, |s| s.elapsed().as_millis() as u64);
                let _ = writeln!(
                    inner.out,
                    "total {} {} (done={} manual={} excluded={} failed={})",
                    format_ms(elapsed),
                    e.status,
                    e.done,
                    e.manual,
                    e.excluded,
                    e.failed
                );
            }
            let hint = (e.manual > 0).then(|| manual_review_hint(e.manual));
            if tty {
                repaint(inner, true);
            }
            if let Some(hint) = hint {
                after(inner, Some(hint), tty, true);
            }
            return;
        }
        _ => return,
    };
    after(inner, line, tty, false);
}

/// Records a tail line and repaints (terminal) or prints it (plain).
fn after(inner: &mut Inner, line: Option<String>, tty: bool, force: bool) {
    if let Some(line) = &line {
        inner.tail.push(line.clone());
        if inner.tail.len() > PANEL_TAIL {
            inner.tail.remove(0);
        }
    }
    if tty {
        repaint(inner, force);
    } else if let Some(line) = line {
        let _ = writeln!(inner.out, "{line}");
    }
}

fn repaint(inner: &mut Inner, force: bool) {
    let now = Instant::now();
    if !force && inner.last_paint.is_some_and(|t| now - t < PANEL_REPAINT) {
        return;
    }
    inner.last_paint = Some(now);
    let mut screen = String::new();
    if inner.drawn > 0 {
        screen.push_str(&format!("\x1b[{}A", inner.drawn));
    }
    let lines = panel_lines(inner);
    for line in &lines {
        screen.push_str("\x1b[2K");
        screen.push_str(line);
        screen.push('\n');
    }
    inner.drawn = lines.len();
    let _ = inner.out.write_all(screen.as_bytes());
}

fn panel_lines(inner: &Inner) -> Vec<String> {
    let elapsed = inner.started.map_or(0, |s| s.elapsed().as_millis() as u64);
    let mut lines = vec![format!(
        "proceso  {} invoice(s)  {}",
        inner.total,
        format_ms(elapsed)
    )];
    for (phase, c) in &inner.counts {
        lines.push(format!(
            "  {phase:<12} in-flight={:<3} done={:<4} manual={:<3} excluded={:<3} failed={:<3}",
            c.inflight, c.done, c.manual, c.excluded, c.failed
        ));
    }
    lines.push(format!("  {:<12} filed={}", "radicacion", inner.filed));
    if !inner.tail.is_empty() {
        lines.push("  recent:".to_owned());
        lines.extend(inner.tail.iter().map(|t| format!("    {t}")));
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    pub struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("buffer").extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Buffer {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().expect("buffer").clone()).expect("utf8")
        }
    }

    fn step(invoice: &str, phase: &str, status: &str) -> Event {
        Event::new("invoice_step")
            .invoice(invoice)
            .phase(phase, "x")
            .status(status)
            .millis(1500)
    }

    #[test]
    fn json_events_use_the_go_field_names_and_omit_zero_values() {
        let out = Buffer::default();
        let events = Events::new(Mode::Json, Box::new(out.clone()), false);
        events.emit(step("FVS1", "fev", "OK").reason("VALOR_DIFERENTE"));
        let mut end = Event::new("run_end").status("OK");
        end.done = 1;
        end.phase_ms.insert("fev".into(), 1500);
        events.emit(end);
        let text = out.text();
        let lines: Vec<&str> = text.lines().collect();
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("json");
        assert_eq!(first["type"], "invoice_step");
        assert_eq!(first["durationMs"], 1500);
        assert_eq!(first["reason"], "VALOR_DIFERENTE");
        assert!(first.get("error").is_none() && first.get("batch").is_none());
        let last: serde_json::Value = serde_json::from_str(lines[1]).expect("json");
        assert_eq!(last["phaseMs"]["fev"], 1500);
        assert!(last.get("skipped").is_none() && last.get("replaced").is_none());
        assert!(last["time"].as_str().is_some_and(|t| t.contains('T')));
    }

    #[test]
    fn text_lines_and_the_summary_follow_go() {
        let out = Buffer::default();
        let events = Events::new(Mode::Text, Box::new(out.clone()), false);
        let mut start = Event::new("run_start");
        start.total = 2;
        events.emit(start);
        events.emit(
            Event::new("invoice_enter")
                .invoice("A1")
                .phase("fev", "fev verificar"),
        );
        events.emit(step("A1", "fev", "MANUAL_REVIEW").reason("XML_SIN_VALOR"));
        let mut end = Event::new("run_end").status("MANUAL_REVIEW");
        end.manual = 1;
        end.replaced = 2;
        end.skipped = 3;
        events.emit(end);
        let text = out.text();
        assert!(text.contains("proceso: 2 invoice(s) entering the pipeline\n"));
        assert!(text.contains("A1           fev          ⚠ MANUAL_REVIEW  1.5s  XML_SIN_VALOR\n"));
        assert!(text.contains(
            "\nproceso: MANUAL_REVIEW (done=0 manual=1 excluded=0 failed=0 replaced=2 skipped=3)\n"
        ));
        assert!(text.contains("  fev          done=0 manual=1 excluded=0 failed=0\n"));
        assert!(text.contains("manual review: 1 invoice(s) to review;"));
    }

    #[test]
    fn the_panel_prints_plain_lines_when_stdout_is_not_a_terminal() {
        let out = Buffer::default();
        let events = Events::new(Mode::Panel, Box::new(out.clone()), false);
        let mut start = Event::new("run_start");
        start.total = 1;
        events.emit(start);
        events.emit(step("A1", "fev", "OK"));
        let mut end = Event::new("run_end").status("OK");
        end.done = 1;
        events.emit(end);
        let text = out.text();
        assert!(!text.contains('\x1b'));
        assert!(text.contains("proceso: 1 invoice(s)\n"));
        assert!(text.contains("A1 fev ✓ 1.5s\n"));
        assert!(text.contains("(done=1 manual=0 excluded=0 failed=0)\n"));
    }

    #[test]
    fn the_panel_repaints_in_place_on_a_terminal() {
        let out = Buffer::default();
        let events = Events::new(Mode::Panel, Box::new(out.clone()), true);
        events.emit(Event::new("convenio_start").step("convenio preparar"));
        events.emit(Event::new("run_end").status("OK"));
        let text = out.text();
        assert!(text.contains("\x1b[2K"));
        assert!(text.contains("radicacion   filed=0"));
    }
}
