//! The embedded UI and the static-serving rules (Go: `desktop_handler.go`,
//! `staticUIHandler`).
//!
//! Resolution is pure (`resolve` over an [`AssetSource`]) so it is testable
//! without a window and without a built UI.

use std::borrow::Cow;
use std::collections::HashMap;

/// The startup error of a build that embeds no UI (same text as Go's).
pub const UI_MISSING: &str =
    "this build embeds no UI (index.html not found); build it with scripts/build-desktop.sh";

/// A read-only set of UI files addressed by slash-separated relative names
/// (`index.html`, `assets/index-ab12.js`).
pub trait AssetSource: Send + Sync {
    fn get(&self, name: &str) -> Option<Cow<'static, [u8]>>;
}

/// An in-memory [`AssetSource`] for tests.
#[derive(Debug, Default, Clone)]
pub struct MemoryAssets(HashMap<String, Vec<u8>>);

impl MemoryAssets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, name: &str, data: impl Into<Vec<u8>>) -> Self {
        self.0.insert(name.to_string(), data.into());
        self
    }
}

impl AssetSource for MemoryAssets {
    fn get(&self, name: &str) -> Option<Cow<'static, [u8]>> {
        self.0.get(name).map(|data| Cow::Owned(data.clone()))
    }
}

/// The UI build (`desktop-ui/dist`) embedded into the executable at compile
/// time. The folder may be missing in a fresh checkout: the source is then
/// empty and [`require_index`] explains why the window cannot open.
#[cfg(feature = "desktop")]
#[derive(rust_embed::RustEmbed)]
#[folder = "desktop-ui/dist"]
#[allow_missing = true]
struct UiFiles;

/// [`AssetSource`] over the embedded UI build.
#[cfg(feature = "desktop")]
#[derive(Debug, Default, Clone, Copy)]
pub struct EmbeddedUi;

#[cfg(feature = "desktop")]
impl AssetSource for EmbeddedUi {
    fn get(&self, name: &str) -> Option<Cow<'static, [u8]>> {
        UiFiles::get(name).map(|file| file.data)
    }
}

/// Fails when the source has no `index.html`: a window with nothing to show
/// must say why instead of opening blank.
pub fn require_index(source: &dyn AssetSource) -> anyhow::Result<()> {
    match source.get("index.html") {
        Some(_) => Ok(()),
        None => anyhow::bail!(UI_MISSING),
    }
}

/// A file picked by [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Name inside the source (`index.html` for an SPA fallback).
    pub name: String,
    pub data: Cow<'static, [u8]>,
}

/// Maps a request path to a UI file, like Go's `staticUIHandler`: an exact
/// file, or a directory's `index.html`; a missing path with an extension is a
/// real miss (`None`); any other unknown path gets the SPA shell (`200.html`,
/// else `index.html`) so client-side routes such as `/ejecuciones` resolve.
pub fn resolve(source: &dyn AssetSource, request_path: &str) -> Option<Asset> {
    let name = clean(request_path);
    for candidate in candidates(&name) {
        if let Some(data) = source.get(&candidate) {
            return Some(Asset {
                name: candidate,
                data,
            });
        }
    }
    if has_extension(&name) {
        return None;
    }
    ["200.html", "index.html"].into_iter().find_map(|shell| {
        source.get(shell).map(|data| Asset {
            name: shell.to_string(),
            data,
        })
    })
}

fn candidates(name: &str) -> Vec<String> {
    if name.is_empty() {
        vec!["index.html".to_string()]
    } else {
        vec![name.to_string(), format!("{name}/index.html")]
    }
}

/// Go's `strings.TrimPrefix(path.Clean("/"+p), "/")`: collapses `//` and `.`,
/// resolves `..` without ever leaving the root.
pub fn clean(request_path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in request_path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Go's `path.Ext(name) != ""`: a dot in the last element (a leading dot counts).
fn has_extension(name: &str) -> bool {
    let last = name.rsplit('/').next().unwrap_or("");
    last.contains('.')
}

/// `Content-Type` of a UI file, with an explicit charset for text.
pub fn content_type(name: &str) -> String {
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let essence = mime.essence_str();
    let textual = mime.type_() == mime_guess::mime::TEXT
        || matches!(
            essence,
            "application/javascript" | "application/json" | "image/svg+xml"
        );
    if textual {
        format!("{essence}; charset=utf-8")
    } else {
        essence.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> MemoryAssets {
        MemoryAssets::new()
            .with("index.html", "<html>shell</html>")
            .with("assets/app.js", "console.log(1)")
            .with("assets/app.css", "a{}")
            .with("logo.png", [0x89, b'P', b'N', b'G'])
            .with("docs/index.html", "<html>docs</html>")
    }

    fn name(source: &dyn AssetSource, path: &str) -> Option<String> {
        resolve(source, path).map(|a| a.name)
    }

    #[test]
    fn root_and_exact_files_resolve() {
        let ui = ui();
        assert_eq!(name(&ui, "/").as_deref(), Some("index.html"));
        assert_eq!(name(&ui, "").as_deref(), Some("index.html"));
        assert_eq!(
            name(&ui, "/assets/app.js").as_deref(),
            Some("assets/app.js")
        );
        assert_eq!(name(&ui, "/logo.png").as_deref(), Some("logo.png"));
    }

    #[test]
    fn a_directory_serves_its_index_with_or_without_slash() {
        let ui = ui();
        assert_eq!(name(&ui, "/docs").as_deref(), Some("docs/index.html"));
        assert_eq!(name(&ui, "/docs/").as_deref(), Some("docs/index.html"));
    }

    #[test]
    fn a_missing_path_with_an_extension_is_a_real_miss() {
        let ui = ui();
        assert_eq!(resolve(&ui, "/assets/missing.js"), None);
        assert_eq!(resolve(&ui, "/favicon.ico"), None);
        assert_eq!(resolve(&ui, "/.hidden"), None);
        assert_eq!(
            resolve(&ui, "/v1.2/ruta"),
            Some(Asset {
                name: "index.html".into(),
                data: Cow::Owned(b"<html>shell</html>".to_vec()),
            })
        );
    }

    #[test]
    fn an_extensionless_unknown_path_falls_back_to_the_shell() {
        let ui = ui();
        for path in ["/ejecuciones", "/revision-manual", "/a/b/c", "/reportes/"] {
            assert_eq!(name(&ui, path).as_deref(), Some("index.html"), "{path}");
        }
    }

    #[test]
    fn a_200_html_shell_wins_over_index_html() {
        let ui = ui().with("200.html", "spa");
        assert_eq!(name(&ui, "/ejecuciones").as_deref(), Some("200.html"));
        assert_eq!(name(&ui, "/").as_deref(), Some("index.html"));
    }

    #[test]
    fn dot_dot_never_leaves_the_root() {
        let ui = ui();
        assert_eq!(clean("/../../etc/passwd"), "etc/passwd");
        assert_eq!(clean("//assets/./app.js"), "assets/app.js");
        // Cleaned to `etc/passwd`: an unknown extension-less path, so the SPA shell.
        assert_eq!(
            name(&ui, "/../../etc/passwd").as_deref(),
            Some("index.html")
        );
        assert_eq!(resolve(&ui, "/../../etc/passwd.txt"), None);
        assert_eq!(
            name(&ui, "/assets/../index.html").as_deref(),
            Some("index.html")
        );
    }

    #[test]
    fn no_shell_means_nothing_to_fall_back_to() {
        let ui = MemoryAssets::new().with("assets/app.js", "x");
        assert_eq!(resolve(&ui, "/ejecuciones"), None);
    }

    #[test]
    fn a_build_without_index_html_fails_with_the_documented_message() {
        let err = require_index(&MemoryAssets::new().with("assets/app.js", "x")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "this build embeds no UI (index.html not found); build it with scripts/build-desktop.sh"
        );
        assert!(require_index(&ui()).is_ok());
    }

    #[test]
    fn content_types_carry_a_charset_for_text_only() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("assets/app.css"), "text/css; charset=utf-8");
        assert_eq!(
            content_type("assets/app.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("logo.png"), "image/png");
        assert_eq!(content_type("blob.unknownext"), "application/octet-stream");
    }
}
