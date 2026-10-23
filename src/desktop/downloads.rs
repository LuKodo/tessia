//! Downloads (`Content-Disposition: attachment` answers, e.g. the CSV reports).
//!
//! The webview asks where to save through wry's download-started handler; the
//! file goes to the OS Downloads folder under a safe, de-duplicated name. The
//! handlers hold no window state, so the naming rules are plain functions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::protocol::is_app_url;

/// Name used when the server suggested nothing usable.
const FALLBACK_NAME: &str = "descarga";

/// Longest file name (in bytes) the shell will create.
const MAX_NAME_BYTES: usize = 150;

/// Device names Windows refuses as file names (with or without extension).
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The OS Downloads folder, or the user's home, or the current directory.
pub fn downloads_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

/// Reduces a suggested name to a plain file name that is valid on every OS: no
/// directories, no control or reserved characters, no trailing dots or spaces,
/// no Windows device names, at most [`MAX_NAME_BYTES`] bytes.
pub fn sanitize_file_name(suggested: &str) -> String {
    let base = suggested.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    let trimmed = trimmed.trim_start_matches('.');
    if trimmed.is_empty() {
        return FALLBACK_NAME.to_string();
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    let reserved = WINDOWS_RESERVED
        .iter()
        .any(|device| device.eq_ignore_ascii_case(stem));
    let named = if reserved {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    };
    truncate_name(&named)
}

/// Cuts a name to [`MAX_NAME_BYTES`] keeping its extension and char boundaries.
fn truncate_name(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_string();
    }
    let (stem, ext) = split_extension(name);
    let ext = if ext.len() <= 16 { ext } else { "" };
    let mut end = MAX_NAME_BYTES - ext.len();
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{ext}", &stem[..end])
}

/// `("informe", ".csv")`; a leading dot is part of the stem.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(dot) => name.split_at(dot),
    }
}

/// The first of `name`, `name (1).ext`, `name (2).ext`, ... that does not exist.
pub fn unique_path(dir: &Path, file_name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(file_name);
    if !exists(&first) {
        return first;
    }
    let (stem, ext) = split_extension(file_name);
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|candidate| !exists(candidate))
        .unwrap_or(first)
}

/// Where a download is saved: `downloads` + the sanitized name of what the
/// webview suggested (`suggested` is `<some dir>/<file name>`), de-duplicated.
pub fn choose_destination(
    suggested: &Path,
    downloads: &Path,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let name = suggested
        .file_name()
        .map(|n| sanitize_file_name(&n.to_string_lossy()))
        .unwrap_or_else(|| FALLBACK_NAME.to_string());
    unique_path(downloads, &name, exists)
}

/// A finished download, for the window to announce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    pub file_name: Option<String>,
    pub success: bool,
}

/// Remembers where each started download goes so the completion handler can
/// name the file even where the webview reports no path (macOS).
#[derive(Debug)]
pub struct Downloads {
    app_origin: &'static str,
    folder: PathBuf,
    saving: HashMap<String, PathBuf>,
}

impl Downloads {
    pub fn new(app_origin: &'static str, folder: PathBuf) -> Self {
        Self {
            app_origin,
            folder,
            saving: HashMap::new(),
        }
    }

    /// wry's download-started handler: only downloads from the app itself are
    /// allowed, saved into the Downloads folder.
    pub fn started(&mut self, url: &str, destination: &mut PathBuf) -> bool {
        if !is_app_url(url, self.app_origin) {
            return false;
        }
        let chosen = choose_destination(destination, &self.folder, |p| p.exists());
        *destination = chosen.clone();
        self.saving.insert(url.to_string(), chosen);
        true
    }

    /// wry's download-completed handler.
    pub fn completed(
        &mut self,
        url: &str,
        path: Option<PathBuf>,
        success: bool,
    ) -> DownloadOutcome {
        let path = path.or_else(|| self.saving.remove(url));
        self.saving.remove(url);
        let file_name = path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned());
        DownloadOutcome { file_name, success }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn existing(names: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = names.iter().map(|n| Path::new("/d").join(n)).collect();
        move |p| set.contains(p)
    }

    #[test]
    fn plain_names_pass_through() {
        assert_eq!(
            sanitize_file_name("informe-revision-manual-Convenio_21102026.csv"),
            "informe-revision-manual-Convenio_21102026.csv"
        );
        assert_eq!(
            sanitize_file_name("Radicación enero.csv"),
            "Radicación enero.csv"
        );
    }

    #[test]
    fn directories_and_reserved_characters_are_removed() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name(r"C:\Windows\evil.exe"), "evil.exe");
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*h.csv"),
            "a_b_c_d_e_f_g_h.csv"
        );
        assert_eq!(sanitize_file_name("tab\tnew\nline.csv"), "tab_new_line.csv");
    }

    #[test]
    fn empty_hidden_and_dotty_names_get_a_usable_name() {
        for name in ["", "   ", "...", "/", ".", ".."] {
            assert_eq!(sanitize_file_name(name), "descarga", "{name:?}");
        }
        assert_eq!(sanitize_file_name(".bashrc"), "bashrc");
        assert_eq!(sanitize_file_name("informe.csv. "), "informe.csv");
    }

    #[test]
    fn windows_device_names_are_prefixed() {
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_file_name("Com1.csv"), "_Com1.csv");
        assert_eq!(sanitize_file_name("console.csv"), "console.csv");
    }

    #[test]
    fn long_names_are_cut_keeping_the_extension_and_chars() {
        let long = format!("{}.csv", "á".repeat(200));
        let cut = sanitize_file_name(&long);
        assert!(cut.len() <= MAX_NAME_BYTES);
        assert!(cut.ends_with(".csv"));
        assert!(cut.starts_with('á'));
    }

    #[test]
    fn a_free_name_is_kept() {
        let path = unique_path(Path::new("/d"), "informe.csv", existing(&[]));
        assert_eq!(path, Path::new("/d/informe.csv"));
    }

    #[test]
    fn a_taken_name_gets_the_first_free_counter_before_the_extension() {
        let taken = existing(&["informe.csv", "informe (1).csv", "informe (2).csv"]);
        assert_eq!(
            unique_path(Path::new("/d"), "informe.csv", taken),
            Path::new("/d/informe (3).csv")
        );
        let gap = existing(&["informe.csv", "informe (2).csv"]);
        assert_eq!(
            unique_path(Path::new("/d"), "informe.csv", gap),
            Path::new("/d/informe (1).csv")
        );
    }

    #[test]
    fn counters_split_at_the_last_dot_and_handle_no_extension() {
        assert_eq!(
            unique_path(Path::new("/d"), "a.b.csv", existing(&["a.b.csv"])),
            Path::new("/d/a.b (1).csv")
        );
        assert_eq!(
            unique_path(Path::new("/d"), "descarga", existing(&["descarga"])),
            Path::new("/d/descarga (1)")
        );
    }

    #[test]
    fn the_suggested_directory_is_replaced_by_downloads() {
        let dest = choose_destination(
            Path::new("/somewhere/else/../x/radicadas.csv"),
            Path::new("/d"),
            existing(&["radicadas.csv"]),
        );
        assert_eq!(dest, Path::new("/d/radicadas (1).csv"));
        let unnamed = choose_destination(Path::new("/"), Path::new("/d"), existing(&[]));
        assert_eq!(unnamed, Path::new("/d/descarga"));
    }

    #[test]
    fn only_downloads_from_the_app_are_allowed() {
        let mut downloads = Downloads::new("tessia://localhost", PathBuf::from("/nonexistent-dir"));
        let mut dest = PathBuf::from("/tmp/x/informe.csv");
        assert!(!downloads.started("https://example.com/informe.csv", &mut dest));
        assert_eq!(
            dest,
            Path::new("/tmp/x/informe.csv"),
            "a denied download keeps its path"
        );

        assert!(downloads.started(
            "tessia://localhost/api/reportes/radicadas?formato=csv",
            &mut dest
        ));
        assert_eq!(dest, Path::new("/nonexistent-dir/informe.csv"));
    }

    #[test]
    fn completion_names_the_file_even_without_a_reported_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut downloads = Downloads::new("tessia://localhost", dir.path().to_path_buf());
        let url = "tessia://localhost/api/cargues/C/informe?formato=csv";

        let mut dest = PathBuf::from("/x/informe.csv");
        assert!(downloads.started(url, &mut dest));
        std::fs::write(&dest, "a").unwrap();
        // macOS reports no path.
        let done = downloads.completed(url, None, true);
        assert_eq!(
            done,
            DownloadOutcome {
                file_name: Some("informe.csv".into()),
                success: true
            }
        );

        // The next download of the same name is de-duplicated.
        let mut again = PathBuf::from("/x/informe.csv");
        assert!(downloads.started(url, &mut again));
        assert_eq!(again, dir.path().join("informe (1).csv"));
        let failed = downloads.completed(url, None, false);
        assert_eq!(
            failed,
            DownloadOutcome {
                file_name: Some("informe (1).csv".into()),
                success: false
            }
        );

        // A path reported by the webview wins.
        let reported = downloads.completed(url, Some(PathBuf::from("/real/place/r.csv")), true);
        assert_eq!(reported.file_name.as_deref(), Some("r.csv"));
    }
}
