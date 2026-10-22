//! Runtime configuration.
//!
//! Precedence (lowest to highest): built-in defaults, an optional
//! `KEY=VALUE` file (`--config <file>` or `TESSIA_CONFIG`), and `TESSIA_*`
//! environment variables. Empty values never override a lower-precedence
//! source. Secrets are never compiled in: they come only from the file or the
//! environment, and are never printed.
//!
//! Every key has a stable dotted name (`cassandra.api_key`) and an environment
//! variable derived from it: `TESSIA_` + the upper-cased key with `.` replaced
//! by `_` (`TESSIA_CASSANDRA_API_KEY`). The file uses the environment names.
//! An unknown name in the file is an error, except the names Go used for
//! machinery this port dropped (see [`RETIRED_ENV_NAMES`]).
//!
//! ```
//! use tessia::config::Settings;
//!
//! let settings = Settings::from_pairs([("TESSIA_REDIS_URL", "redis://localhost")]);
//! assert_eq!(settings.get("redis.url"), Some("redis://localhost"));
//! assert_eq!(settings.get("redis.nt_prefix"), Some("nt:")); // default
//! assert!(settings.require(&["database.url"]).is_err());
//! ```

pub mod cli;
mod envfile;
mod keys;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub use keys::{KEYS, KeyDef, RETIRED_ENV_NAMES, env_name};

/// Environment variable naming the `KEY=VALUE` file.
pub const ENV_CONFIG_FILE: &str = "TESSIA_CONFIG";

/// Errors of [`Settings::require`] and loading.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("faltan claves de configuración requeridas: {}", .0.join(", "))]
    Missing(Vec<String>),
    #[error("clave de configuración desconocida: {}", .0.join(", "))]
    UnknownKey(Vec<String>),
}

/// Whether a key has a value, never the value itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStatus {
    pub key: &'static str,
    pub env: String,
    pub set: bool,
    pub secret: bool,
}

/// Immutable configuration values. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    values: BTreeMap<&'static str, String>,
    file: Option<PathBuf>,
}

impl Settings {
    /// Loads defaults + `file` (or `TESSIA_CONFIG`) + the process environment.
    pub fn load(file: Option<&Path>) -> Result<Settings> {
        let environ: Vec<(String, String)> = std::env::vars().collect();
        Self::load_with(file, &environ)
    }

    /// Like [`Settings::load`] with an explicit environment (tests).
    pub fn load_with(file: Option<&Path>, environ: &[(String, String)]) -> Result<Settings> {
        let env: BTreeMap<&str, &str> = environ
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let mut settings = Settings::default();
        let file = file.map(Path::to_path_buf).or_else(|| {
            env.get(ENV_CONFIG_FILE)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        });
        if let Some(path) = file {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("config: no se pudo leer {}", path.display()))?;
            let pairs =
                envfile::parse(&raw).with_context(|| format!("config: {}", path.display()))?;
            for (name, value) in pairs {
                match keys::by_env_name(&name) {
                    Some(def) => {
                        if !value.is_empty() {
                            settings.values.insert(def.key, value);
                        }
                    }
                    None if RETIRED_ENV_NAMES.contains(&name.as_str()) => {}
                    None => {
                        return Err(ConfigError::UnknownKey(vec![format!(
                            "{name} en {}",
                            path.display()
                        )])
                        .into());
                    }
                }
            }
            settings.file = Some(path);
        }
        for def in KEYS {
            if let Some(value) = env
                .get(env_name(def.key).as_str())
                .filter(|v| !v.is_empty())
            {
                settings.values.insert(def.key, (*value).to_owned());
            }
        }
        Ok(settings)
    }

    /// Builds settings from `(ENV_NAME, value)` pairs; unknown names are
    /// ignored. Meant for tests and embedding.
    pub fn from_pairs<I, K, V>(pairs: I) -> Settings
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut settings = Settings::default();
        for (name, value) in pairs {
            if let Some(def) = keys::by_env_name(name.as_ref()) {
                if !value.as_ref().is_empty() {
                    settings.values.insert(def.key, value.as_ref().to_owned());
                }
            }
        }
        settings
    }

    /// Returns a copy with `key` set to `value` (tests, overrides). Unknown
    /// keys are ignored; an empty value removes the explicit value.
    #[must_use]
    pub fn with(mut self, key: &str, value: &str) -> Settings {
        if let Some(def) = keys::by_key(key) {
            if value.is_empty() {
                self.values.remove(def.key);
            } else {
                self.values.insert(def.key, value.to_owned());
            }
        }
        self
    }

    /// Value of a dotted key: the explicit value, else its default, else
    /// `None`. Unknown keys also return `None`.
    pub fn get(&self, key: &str) -> Option<&str> {
        let def = keys::by_key(key)?;
        self.values
            .get(def.key)
            .map(String::as_str)
            .or(def.default)
            .filter(|v| !v.trim().is_empty())
    }

    /// Value of `key` or `fallback` when unset.
    pub fn get_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.get(key).unwrap_or(fallback)
    }

    /// True when `key` has a non-blank value (explicit or default).
    pub fn is_set(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The value of `key`, or an error naming the key and its environment
    /// variable (never a value).
    pub fn require_one(&self, key: &str) -> Result<&str, ConfigError> {
        self.require(&[key])?;
        self.get(key)
            .ok_or_else(|| ConfigError::Missing(vec![key.to_owned()]))
    }

    /// Fails when any key is unknown or has no value. The error names keys and
    /// environment variables, never values.
    pub fn require(&self, keys: &[&str]) -> Result<(), ConfigError> {
        let mut unknown: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for key in keys {
            if keys::by_key(key).is_none() {
                unknown.push((*key).to_owned());
            } else if !self.is_set(key) {
                missing.push(format!("{key} ({})", env_name(key)));
            }
        }
        if !unknown.is_empty() {
            unknown.sort();
            return Err(ConfigError::UnknownKey(unknown));
        }
        if !missing.is_empty() {
            return Err(ConfigError::Missing(missing));
        }
        Ok(())
    }

    /// Every key's status in documentation order, without values.
    pub fn describe(&self) -> Vec<KeyStatus> {
        KEYS.iter()
            .map(|def| KeyStatus {
                key: def.key,
                env: env_name(def.key),
                set: self.is_set(def.key),
                secret: def.secret,
            })
            .collect()
    }

    /// The configuration file that was loaded, if any.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Working directory root (`workdir.root`, default `<os temp>/tessia-work`).
    pub fn workdir_root(&self) -> PathBuf {
        self.get("workdir.root")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("tessia-work"))
    }

    /// Comma-separated value split into trimmed, non-empty items.
    pub fn list(&self, key: &str) -> Vec<String> {
        self.get(key)
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn defaults_apply_when_nothing_is_set() {
        let s = Settings::load_with(None, &[]).expect("load");
        assert_eq!(s.get("company.nit_emisor"), Some("806011811"));
        assert_eq!(s.get("redis.nt_prefix"), Some("nt:"));
        assert_eq!(s.get("database.url"), None);
        assert!(!s.is_set("cassandra.api_key"));
    }

    #[test]
    fn environment_beats_file_beats_defaults_and_empty_never_overrides() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("c.env");
        std::fs::write(
            &file,
            "# comment\nTESSIA_COMPANY_NIT_EMISOR=111\nexport TESSIA_REDIS_URL=\"redis://file\"\nTESSIA_CASSANDRA_BUCKET=''\n",
        )
        .expect("write");
        let environ = env(&[
            ("TESSIA_REDIS_URL", "redis://env"),
            ("TESSIA_COMPANY_NIT_EMISOR", ""),
        ]);
        let s = Settings::load_with(Some(&file), &environ).expect("load");
        assert_eq!(s.get("redis.url"), Some("redis://env"));
        assert_eq!(s.get("company.nit_emisor"), Some("111"));
        assert_eq!(s.get("cassandra.bucket"), None);
        assert_eq!(s.file(), Some(file.as_path()));
    }

    #[test]
    fn config_file_can_come_from_tessia_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("c.env");
        std::fs::write(&file, "TESSIA_DATABASE_URL=postgres://x\n").expect("write");
        let environ = env(&[("TESSIA_CONFIG", file.to_str().expect("utf8"))]);
        let s = Settings::load_with(None, &environ).expect("load");
        assert_eq!(s.get("database.url"), Some("postgres://x"));
    }

    #[test]
    fn unknown_key_in_file_is_an_error_but_retired_ones_are_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("c.env");
        std::fs::write(
            &file,
            "TESSIA_MYSQL_TESSIA_DSN=user:pw@tcp(h)/db\nTESSIA_PATHS_SIESA=x\n",
        )
        .expect("write");
        assert!(Settings::load_with(Some(&file), &[]).is_ok());
        std::fs::write(&file, "TESSIA_NOPE=1\n").expect("write");
        let err = Settings::load_with(Some(&file), &[]).expect_err("unknown key");
        assert!(format!("{err:#}").contains("TESSIA_NOPE"), "{err:#}");
    }

    #[test]
    fn require_names_missing_keys_without_values() {
        let s = Settings::from_pairs([("TESSIA_DATABASE_URL", "postgres://secret")]);
        let err = s
            .require(&["database.url", "redis.url", "cassandra.base_url"])
            .expect_err("missing");
        let text = err.to_string();
        assert!(text.contains("redis.url (TESSIA_REDIS_URL)"), "{text}");
        assert!(
            text.contains("cassandra.base_url (TESSIA_CASSANDRA_BASE_URL)"),
            "{text}"
        );
        assert!(!text.contains("secret"));
        assert!(matches!(
            s.require(&["nope.key"]),
            Err(ConfigError::UnknownKey(_))
        ));
    }

    #[test]
    fn describe_flags_secrets_and_never_exposes_values() {
        let s = Settings::from_pairs([("TESSIA_CASSANDRA_API_KEY", "k")]);
        let status = s
            .describe()
            .into_iter()
            .find(|k| k.key == "cassandra.api_key")
            .expect("key");
        assert!(status.set && status.secret);
        assert_eq!(status.env, "TESSIA_CASSANDRA_API_KEY");
    }

    #[test]
    fn workdir_root_defaults_to_the_os_temp_dir() {
        let s = Settings::default();
        assert_eq!(s.workdir_root(), std::env::temp_dir().join("tessia-work"));
        assert_eq!(
            s.with("workdir.root", "/x").workdir_root(),
            PathBuf::from("/x")
        );
    }
}
