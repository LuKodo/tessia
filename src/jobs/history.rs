//! Finished jobs, persisted as JSON lines so they survive a restart (Go
//! `historyEntry`, `newJobRunner`, `recordHistory`).
//!
//! The file is append-only and never trimmed; only the last [`MAX_ENTRIES`]
//! are loaded, newest first. It lives under the OS user data directory (the
//! history is per machine), not next to the cargues.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use super::snapshot::{JobSnapshot, JobStats};
use crate::util::fsx;

/// The file name Go used (`<paths.automatic>/.tessia-serve-jobs.jsonl`).
pub const FILE_NAME: &str = ".tessia-serve-jobs.jsonl";
/// Output kept per history entry.
pub const MAX_TAIL_BYTES: usize = 2 << 10;
/// Past jobs kept in memory and shown.
pub const MAX_ENTRIES: usize = 50;
/// Longest history line the loader accepts.
const MAX_LINE_BYTES: usize = 1 << 20;

/// A finished job's audit record. `error`, `salida_final` and `estadisticas`
/// are omitted when empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HistoryEntry {
    pub id: String,
    pub tipo: String,
    pub iniciado_en: String,
    pub terminado_en: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub salida_final: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estadisticas: Option<JobStats>,
}

impl HistoryEntry {
    /// The record of a finished job's snapshot.
    pub fn from_snapshot(snapshot: &JobSnapshot) -> Self {
        Self {
            id: snapshot.id.clone(),
            tipo: snapshot.tipo.clone(),
            iniciado_en: snapshot.iniciado_en.clone(),
            terminado_en: snapshot.terminado_en.clone(),
            error: snapshot.error.clone(),
            salida_final: tail(&snapshot.salida, MAX_TAIL_BYTES).to_owned(),
            estadisticas: snapshot.estadisticas.clone(),
        }
    }
}

/// The last `max` bytes of `text`, moved forward to a character boundary so the
/// cut never splits a multi-byte character.
pub fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut at = text.len() - max;
    while !text.is_char_boundary(at) {
        at += 1;
    }
    &text[at..]
}

/// The OS user data directory of the app: `~/Library/Application Support/Tessia`
/// on macOS, `%APPDATA%\Tessia` on Windows, `$XDG_DATA_HOME/tessia` (or
/// `~/.local/share/tessia`) elsewhere. `None` when the home directory is unknown.
pub fn default_dir() -> Option<PathBuf> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(target_os = "windows") {
        let base = env("APPDATA")
            .or_else(|| env("USERPROFILE").map(|p| p.join("AppData").join("Roaming")))?;
        Some(base.join("Tessia"))
    } else if cfg!(target_os = "macos") {
        Some(
            env("HOME")?
                .join("Library")
                .join("Application Support")
                .join("Tessia"),
        )
    } else {
        let base =
            env("XDG_DATA_HOME").or_else(|| env("HOME").map(|p| p.join(".local").join("share")))?;
        Some(base.join("tessia"))
    }
}

/// What a startup read found.
#[derive(Debug, Default)]
pub struct Loaded {
    /// Newest first, at most [`MAX_ENTRIES`].
    pub entries: Vec<HistoryEntry>,
    /// Highest `N` among every `job-N` id read.
    pub max_seq: u64,
}

/// The history file (or memory only when there is no directory).
#[derive(Debug, Clone, Default)]
pub struct HistoryFile {
    path: Option<PathBuf>,
}

impl HistoryFile {
    /// Persists in `<dir>/.tessia-serve-jobs.jsonl`; `None` keeps no file.
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            path: dir.map(|dir| dir.join(FILE_NAME)),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Reads the file, best effort: a missing or corrupt file starts empty and
    /// unparsable (or over-long) lines are skipped; it never blocks startup.
    pub fn load(&self) -> Loaded {
        let Some(file) = self.path.as_deref().and_then(|p| File::open(p).ok()) else {
            return Loaded::default();
        };
        let mut all: Vec<HistoryEntry> = Vec::new();
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if line.len() > MAX_LINE_BYTES {
                continue;
            }
            if let Some(entry) = parse_line(&line) {
                all.push(entry);
            }
        }
        let max_seq = all.iter().filter_map(|e| seq_of(&e.id)).max().unwrap_or(0);
        all.reverse(); // the file is oldest first, the history newest first
        all.truncate(MAX_ENTRIES);
        Loaded {
            entries: all,
            max_seq,
        }
    }

    /// Appends one line (mode `0600`). A failure is returned for the caller to
    /// log: losing the audit trail must never break the dashboard.
    pub fn append(&self, entry: &HistoryEntry) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fsx::ensure_dir(parent)?;
        }
        let mut line = serde_json::to_vec(entry).context("no se pudo serializar el historial")?;
        line.push(b'\n');
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(path)
            .with_context(|| format!("no se pudo abrir {}", path.display()))?;
        file.write_all(&line)
            .with_context(|| format!("no se pudo escribir en {}", path.display()))
    }
}

fn parse_line(raw: &[u8]) -> Option<HistoryEntry> {
    let entry: HistoryEntry = serde_json::from_slice(raw.trim_ascii()).ok()?;
    let valid_time = |t: &str| t.is_empty() || DateTime::parse_from_rfc3339(t).is_ok();
    (valid_time(&entry.iniciado_en) && valid_time(&entry.terminado_en)).then_some(entry)
}

/// `N` of a `job-N` id.
pub fn seq_of(id: &str) -> Option<u64> {
    id.strip_prefix("job-")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u64) -> HistoryEntry {
        HistoryEntry {
            id: format!("job-{n}"),
            tipo: "proceso run".into(),
            iniciado_en: "2026-10-22T09:00:01-05:00".into(),
            terminado_en: "2026-10-22T09:03:13.5-05:00".into(),
            ..HistoryEntry::default()
        }
    }

    #[test]
    fn the_tail_is_cut_on_a_character_boundary() {
        assert_eq!(tail("hello", 10), "hello");
        assert_eq!(tail("hello", 3), "llo");
        // "ñ" is two bytes: a cut inside it moves forward, never splits it.
        let text = "ñññ";
        assert_eq!(text.len(), 6);
        assert_eq!(tail(text, 5), "ññ");
        assert_eq!(tail(text, 4), "ññ");
        assert_eq!(tail(text, 3), "ñ");
        assert_eq!(tail(text, 0), "");
        let long = "á".repeat(3000);
        let cut = tail(&long, MAX_TAIL_BYTES);
        assert!(cut.len() <= MAX_TAIL_BYTES && cut.len() >= MAX_TAIL_BYTES - 1);
        assert!(cut.chars().all(|c| c == 'á'));
    }

    #[test]
    fn an_entry_omits_its_empty_optional_fields() {
        let json = serde_json::to_string(&entry(1)).expect("json");
        assert_eq!(
            json,
            r#"{"id":"job-1","tipo":"proceso run","iniciado_en":"2026-10-22T09:00:01-05:00","terminado_en":"2026-10-22T09:03:13.5-05:00"}"#
        );
        let full = HistoryEntry {
            error: "boom".into(),
            salida_final: "out".into(),
            estadisticas: Some(JobStats::default()),
            ..entry(2)
        };
        let json = serde_json::to_value(&full).expect("json");
        assert_eq!(json["error"], "boom");
        assert_eq!(json["salida_final"], "out");
        assert_eq!(json["estadisticas"]["revision_manual"], 0);
    }

    #[test]
    fn the_last_fifty_load_newest_first_and_ids_continue_after_the_highest() {
        let dir = tempfile::tempdir().expect("dir");
        let history = HistoryFile::new(Some(dir.path().to_path_buf()));
        for n in 1..=60 {
            history.append(&entry(n)).expect("append");
        }
        let loaded = history.load();
        assert_eq!(loaded.entries.len(), MAX_ENTRIES);
        assert_eq!(loaded.entries[0].id, "job-60");
        assert_eq!(loaded.entries[49].id, "job-11");
        assert_eq!(loaded.max_seq, 60);
    }

    #[test]
    fn unparsable_lines_are_skipped_and_the_file_is_private() {
        let dir = tempfile::tempdir().expect("dir");
        let history = HistoryFile::new(Some(dir.path().to_path_buf()));
        history.append(&entry(1)).expect("append");
        let path = dir.path().join(FILE_NAME);
        let mut file = OpenOptions::new().append(true).open(&path).expect("open");
        writeln!(file, "not json").expect("write");
        writeln!(file, r#"{{"id":"job-7","iniciado_en":"yesterday"}}"#).expect("write");
        writeln!(file).expect("write");
        writeln!(file, r#"{{"id":"job-3","tipo":"x"}}"#).expect("write");
        let loaded = history.load();
        let ids: Vec<&str> = loaded.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["job-3", "job-1"]);
        assert_eq!(loaded.max_seq, 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_missing_file_or_directory_starts_empty_and_creates_the_directory_on_append() {
        let dir = tempfile::tempdir().expect("dir");
        let nested = dir.path().join("a").join("b");
        let history = HistoryFile::new(Some(nested.clone()));
        let loaded = history.load();
        assert!(loaded.entries.is_empty());
        assert_eq!(loaded.max_seq, 0);
        history.append(&entry(1)).expect("append");
        assert!(nested.join(FILE_NAME).is_file());
        // No directory configured: memory only, nothing to read or write.
        let memory = HistoryFile::new(None);
        memory.append(&entry(1)).expect("noop");
        assert!(memory.load().entries.is_empty());
    }

    #[test]
    fn a_line_over_one_mebibyte_is_skipped() {
        let dir = tempfile::tempdir().expect("dir");
        let history = HistoryFile::new(Some(dir.path().to_path_buf()));
        history.append(&entry(1)).expect("append");
        let huge = HistoryEntry {
            salida_final: "x".repeat(MAX_LINE_BYTES + 10),
            ..entry(2)
        };
        history.append(&huge).expect("append");
        history.append(&entry(3)).expect("append");
        let ids: Vec<String> = history.load().entries.into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["job-3", "job-1"]);
    }

    #[test]
    fn the_entry_of_a_snapshot_keeps_the_output_tail() {
        let snapshot = JobSnapshot {
            id: "job-4".into(),
            tipo: "convenio descargar".into(),
            salida: format!("{}fin", "ñ".repeat(2000)),
            error: "e".into(),
            ..JobSnapshot::idle("i")
        };
        let entry = HistoryEntry::from_snapshot(&snapshot);
        assert!(entry.salida_final.len() <= MAX_TAIL_BYTES);
        assert!(entry.salida_final.ends_with("ñfin"));
        assert_eq!((entry.id.as_str(), entry.error.as_str()), ("job-4", "e"));
    }

    #[test]
    fn ids_parse_only_in_the_job_n_form() {
        assert_eq!(seq_of("job-12"), Some(12));
        for bad in ["job-", "job-x", "x-1", "", "job--1"] {
            assert_eq!(seq_of(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_default_directory_is_under_the_user_data_dir() {
        // Only the shape is asserted: the environment decides the prefix.
        if let Some(dir) = default_dir() {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_lowercase();
            assert_eq!(name, "tessia");
        }
    }
}
