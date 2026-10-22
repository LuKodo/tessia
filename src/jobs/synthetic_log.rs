//! The snapshot's `salida`: a short synthetic Spanish log built from the run's
//! events (Go `synthetic_log.go`). The dashboard does not stream the console:
//! it gets the start line, one summary per finished phase, the first few
//! errors, the manual-review pointer and the result.

use std::time::Duration;

use chrono::{DateTime, Local};

use super::snapshot::{CONVENIO_ERROR, CONVENIO_OK, JobConvenio, JobStats, PhaseProgress};
use crate::phases::proceso::steps::ORDER;

/// Longest error line / final failure message, in characters.
pub const MAX_ERROR_REASON_CHARS: usize = 160;
pub const MAX_RESULT_ERROR_CHARS: usize = 240;

/// One failed invoice (or radicación batch) as the log lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError {
    pub invoice: String,
    pub phase: String,
    pub reason: String,
}

/// Everything [`build`] needs, borrowed from a job under its lock so the
/// builder stays pure.
pub struct SyntheticInput<'a> {
    pub kind: &'a str,
    pub cargue: &'a str,
    /// Labels of the phases the run was asked to execute.
    pub plan: &'a [String],
    pub started_at: DateTime<Local>,
    pub ended_at: Option<DateTime<Local>>,
    pub now: DateTime<Local>,
    pub running: bool,
    pub cancelled: bool,
    pub error: &'a str,
    pub convenio: Option<&'a JobConvenio>,
    pub stats: Option<&'a JobStats>,
    pub phases: &'a [PhaseProgress],
    /// The pipeline reported `run_end`.
    pub run_ended: bool,
    /// Invoices left for manual review so far.
    pub manual: usize,
    pub errors: &'a [JobError],
    /// Distinct errors beyond `errors`.
    pub errors_more: usize,
}

/// Renders the log. While the job runs it only has the start line (and the
/// convenio result once step 0 settled); the phase summaries, errors and the
/// result appear when the run is over.
pub fn build(input: &SyntheticInput) -> String {
    let mut out: Vec<String> = Vec::new();

    let mut start = format!(
        "Inicio {} · {}",
        input.started_at.format("%H:%M:%S"),
        input.kind
    );
    if !input.cargue.is_empty() {
        start.push_str(" · cargue ");
        start.push_str(input.cargue);
    }
    if !input.plan.is_empty() {
        start.push_str(" · fases: ");
        start.push_str(&input.plan.join(" → "));
    }
    out.push(start);

    if let Some(convenio) = input.convenio {
        match convenio.estado.as_str() {
            CONVENIO_OK => out.push(format!(
                "Convenio preparar: {} preparadas · {} excluidas · {} revisión manual · {} evaluadas",
                convenio.preparadas, convenio.excluidas, convenio.revision_manual, convenio.evaluadas
            )),
            CONVENIO_ERROR => out.push(format!(
                "Convenio preparar: error · {}",
                one_line(&convenio.mensaje, MAX_RESULT_ERROR_CHARS)
            )),
            _ => {}
        }
    }

    if input.running && !input.run_ended {
        return out.join("\n");
    }

    let end = if input.running {
        input.now
    } else {
        input.ended_at.unwrap_or(input.now)
    };
    let elapsed = format_elapsed(to_duration(end - input.started_at));

    for phase in sorted_phases(input.phases) {
        let mut line = format!(
            "{}: {} ok · {} revisión manual · {} con errores",
            upper_first(&phase.etiqueta),
            phase.terminadas,
            phase.revision_manual,
            phase.con_errores
        );
        if phase.excluidas > 0 {
            line.push_str(&format!(" · {} excluidas", phase.excluidas));
        }
        out.push(format!(
            "{line} · {}",
            format_elapsed(Duration::from_millis(phase.tiempo_ms))
        ));
    }

    for error in input.errors {
        out.push(format!(
            "Error · {} · {} · {}",
            error.invoice,
            error.phase,
            one_line(&error.reason, MAX_ERROR_REASON_CHARS)
        ));
    }
    match input.errors_more {
        0 => {}
        1 => out.push("+1 error más".to_owned()),
        more => out.push(format!("+{more} errores más")),
    }

    if input.manual > 0 {
        let noun = if input.manual == 1 {
            "factura"
        } else {
            "facturas"
        };
        out.push(format!(
            "Revisión manual: {} {noun}; ábrelas en el \"Informe de revisión manual\".",
            input.manual
        ));
    }

    if input.running {
        return out.join("\n"); // run_end seen, the job is still closing
    }
    let subject = if input.kind == "proceso run" {
        "Proceso".to_owned()
    } else {
        upper_first(input.kind)
    };
    out.push(match (input.error.is_empty(), input.cancelled) {
        (false, true) => format!("{subject} cancelado por el operador tras {elapsed}"),
        (false, false) => format!(
            "{subject} con error tras {elapsed}: {}",
            one_line(input.error, MAX_RESULT_ERROR_CHARS)
        ),
        (true, _) => format!("{subject} terminado en {elapsed}{}", stats_suffix(input)),
    });
    out.join("\n")
}

/// The `: 110 terminadas · ...` tail of the result line, only once the
/// pipeline reported its final tally.
fn stats_suffix(input: &SyntheticInput) -> String {
    let Some(stats) = input.stats.filter(|_| input.run_ended) else {
        return String::new();
    };
    let mut parts = vec![
        format!("{} terminadas", stats.terminadas),
        format!("{} con errores", stats.con_errores),
        format!("{} revisión manual", stats.revision_manual),
    ];
    if stats.excluidas > 0 {
        parts.push(format!("{} excluidas", stats.excluidas));
    }
    if stats.reemplazadas > 0 {
        parts.push(format!("{} reemplazadas", stats.reemplazadas));
    }
    format!(": {}", parts.join(" · "))
}

/// Phase tallies in the canonical pipeline order (unknown phases last, in
/// their given order).
fn sorted_phases(phases: &[PhaseProgress]) -> Vec<&PhaseProgress> {
    let rank = |name: &str| ORDER.iter().position(|s| *s == name).unwrap_or(ORDER.len());
    let mut out: Vec<&PhaseProgress> = phases.iter().collect();
    out.sort_by_key(|p| rank(&p.fase));
    out
}

fn to_duration(delta: chrono::TimeDelta) -> Duration {
    delta.to_std().unwrap_or(Duration::ZERO)
}

/// `45 s`, `3 min 12 s`, `1 h 05 min` (whole seconds, never negative).
pub fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours} h {minutes:02} min")
    } else if minutes > 0 {
        format!("{minutes} min {seconds} s")
    } else {
        format!("{seconds} s")
    }
}

/// Collapses all whitespace (newlines included) into single spaces and cuts the
/// result to `max` characters, marking the cut with an ellipsis.
pub fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > max {
        let cut: String = joined.chars().take(max).collect();
        format!("{cut}…")
    } else {
        joined
    }
}

fn upper_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(h: u32, m: u32, s: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 22, h, m, s)
            .single()
            .expect("local time")
    }

    fn phase(
        key: &str,
        label: &str,
        ok: usize,
        manual: usize,
        failed: usize,
        ms: u64,
    ) -> PhaseProgress {
        PhaseProgress {
            fase: key.into(),
            etiqueta: label.into(),
            terminadas: ok,
            revision_manual: manual,
            con_errores: failed,
            tiempo_ms: ms,
            ..PhaseProgress::default()
        }
    }

    fn input<'a>(plan: &'a [String], phases: &'a [PhaseProgress]) -> SyntheticInput<'a> {
        SyntheticInput {
            kind: "proceso run",
            cargue: "C1",
            plan,
            started_at: at(9, 0, 1),
            ended_at: Some(at(9, 3, 13)),
            now: at(9, 1, 0),
            running: false,
            cancelled: false,
            error: "",
            convenio: None,
            stats: None,
            phases,
            run_ended: true,
            manual: 0,
            errors: &[],
            errors_more: 0,
        }
    }

    #[test]
    fn elapsed_reads_seconds_minutes_and_hours() {
        let secs = Duration::from_secs;
        assert_eq!(format_elapsed(secs(0)), "0 s");
        assert_eq!(format_elapsed(secs(45)), "45 s");
        assert_eq!(format_elapsed(secs(192)), "3 min 12 s");
        assert_eq!(format_elapsed(secs(3900)), "1 h 05 min");
        assert_eq!(format_elapsed(Duration::from_millis(1999)), "1 s");
    }

    #[test]
    fn one_line_collapses_whitespace_and_cuts_on_characters() {
        assert_eq!(one_line("  a\n b\t\tc ", 10), "a b c");
        assert_eq!(one_line("ñandú", 3), "ñan…");
        assert_eq!(one_line("abc", 3), "abc");
    }

    #[test]
    fn a_running_job_has_only_the_start_line_and_the_convenio_result() {
        let plan = vec!["convenio preparar".to_owned(), "fev verificar".to_owned()];
        let convenio = JobConvenio {
            estado: "ok".into(),
            preparadas: 10,
            excluidas: 1,
            revision_manual: 2,
            evaluadas: 13,
            ..JobConvenio::default()
        };
        let mut running = input(&plan, &[]);
        running.running = true;
        running.run_ended = false;
        running.ended_at = None;
        running.convenio = Some(&convenio);
        assert_eq!(
            build(&running),
            "Inicio 09:00:01 · proceso run · cargue C1 · fases: convenio preparar → fev verificar\n\
             Convenio preparar: 10 preparadas · 1 excluidas · 2 revisión manual · 13 evaluadas"
        );
        let failed = JobConvenio {
            estado: "error".into(),
            mensaje: "sin\nnota técnica".into(),
            ..JobConvenio::default()
        };
        running.convenio = Some(&failed);
        assert!(build(&running).ends_with("Convenio preparar: error · sin nota técnica"));
    }

    #[test]
    fn a_finished_run_lists_phases_in_pipeline_order_errors_and_the_result() {
        let phases = vec![
            phase("cuv", "cuv generar", 5, 0, 1, 12_000),
            phase("fev", "fev verificar", 6, 1, 0, 65_000),
        ];
        let errors = vec![JobError {
            invoice: "F1".into(),
            phase: "cuv generar".into(),
            reason: "boom".into(),
        }];
        let stats = JobStats {
            terminadas: 5,
            con_errores: 1,
            revision_manual: 1,
            excluidas: 2,
            reemplazadas: 3,
            ..JobStats::default()
        };
        let mut finished = input(&[], &phases);
        finished.errors = &errors;
        finished.errors_more = 2;
        finished.manual = 1;
        finished.stats = Some(&stats);
        assert_eq!(
            build(&finished),
            "Inicio 09:00:01 · proceso run · cargue C1\n\
             Fev verificar: 6 ok · 1 revisión manual · 0 con errores · 1 min 5 s\n\
             Cuv generar: 5 ok · 0 revisión manual · 1 con errores · 12 s\n\
             Error · F1 · cuv generar · boom\n\
             +2 errores más\n\
             Revisión manual: 1 factura; ábrelas en el \"Informe de revisión manual\".\n\
             Proceso terminado en 3 min 12 s: 5 terminadas · 1 con errores · 1 revisión manual · 2 excluidas · 3 reemplazadas"
        );
        finished.errors_more = 1;
        finished.manual = 4;
        assert!(build(&finished).contains("+1 error más\nRevisión manual: 4 facturas;"));
    }

    #[test]
    fn the_result_line_distinguishes_cancel_error_and_other_kinds() {
        let mut done = input(&[], &[]);
        done.error = "interrumpido";
        done.cancelled = true;
        assert!(build(&done).ends_with("Proceso cancelado por el operador tras 3 min 12 s"));
        done.cancelled = false;
        done.error = "algo\nfalló";
        assert!(build(&done).ends_with("Proceso con error tras 3 min 12 s: algo falló"));
        done.error = "";
        done.kind = "convenio descargar";
        done.run_ended = false;
        assert!(build(&done).ends_with("Convenio descargar terminado en 3 min 12 s"));
        done.kind = "conciliar invoice FEV1";
        assert!(build(&done).contains("Inicio 09:00:01 · conciliar invoice FEV1"));
    }

    #[test]
    fn a_closing_job_that_saw_run_end_shows_the_summaries_but_no_result() {
        let phases = vec![phase("fev", "fev verificar", 1, 0, 0, 1000)];
        let mut closing = input(&[], &phases);
        closing.running = true;
        closing.ended_at = None;
        closing.now = at(9, 0, 31);
        let log = build(&closing);
        assert!(log.contains("Fev verificar: 1 ok"), "{log}");
        assert!(!log.contains("terminado"), "{log}");
    }
}
