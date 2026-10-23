//! Blocking HTTP client builder.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::blocking::Client;

/// Default whole-request timeout (Go `DefaultTimeout`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// Default connect timeout (Go `DefaultDialTimeout`).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Idle pooled connections are dropped after this long (Go 90 s).
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Idle pooled connections kept per host (Go 16).
pub const DEFAULT_MAX_IDLE_PER_HOST: usize = 16;

/// Options of [`build_client`]; the default is a plain client with the Go
/// defaults.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Bounds a whole request including reading the body.
    pub timeout: Duration,
    pub connect_timeout: Duration,
    /// Trusts exactly the PEM certificates in this file (the system roots are
    /// NOT used), like the `*.ca_file` settings.
    pub ca_file: Option<PathBuf>,
    /// Keep cookies across requests (JSF portals, Medicar).
    pub cookies: bool,
    /// Sent on every request (see `MEDICAR_USER_AGENT`).
    pub user_agent: Option<String>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            ca_file: None,
            cookies: false,
            user_agent: None,
        }
    }
}

impl ClientOptions {
    #[must_use]
    pub fn with_ca_file(mut self, path: Option<&str>) -> Self {
        self.ca_file = path.filter(|p| !p.is_empty()).map(PathBuf::from);
        self
    }

    #[must_use]
    pub fn with_cookies(mut self) -> Self {
        self.cookies = true;
        self
    }

    #[must_use]
    pub fn with_user_agent(mut self, agent: &str) -> Self {
        self.user_agent = Some(agent.to_owned());
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Builds a blocking client (rustls, gzip, proxy from the environment).
pub fn build_client(options: &ClientOptions) -> Result<Client> {
    let mut builder = Client::builder()
        .timeout(options.timeout)
        .connect_timeout(options.connect_timeout)
        .pool_idle_timeout(DEFAULT_IDLE_TIMEOUT)
        .pool_max_idle_per_host(DEFAULT_MAX_IDLE_PER_HOST)
        .cookie_store(options.cookies);
    if let Some(agent) = &options.user_agent {
        builder = builder.user_agent(agent.as_str());
    }
    if let Some(path) = &options.ca_file {
        let pem = std::fs::read(path)
            .with_context(|| format!("no se pudo leer el archivo CA {}", path.display()))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem).with_context(|| {
            format!(
                "el archivo CA no contiene certificados PEM válidos: {}",
                path.display()
            )
        })?;
        if certs.is_empty() {
            anyhow::bail!(
                "el archivo CA no contiene certificados PEM: {}",
                path.display()
            );
        }
        builder = builder.tls_built_in_root_certs(false);
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder.build().context("no se pudo crear el cliente HTTP")
}

/// A URL without userinfo, query or fragment (they may carry credentials or
/// signed tokens): safe to log and to put in error messages.
pub fn safe_url(url: &reqwest::Url) -> String {
    let mut clean = url.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.set_query(None);
    clean.set_fragment(None);
    clean.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_url_drops_credentials_query_and_fragment() {
        let url = reqwest::Url::parse("https://u:p@h.example/a/b?token=s#frag").expect("url");
        assert_eq!(safe_url(&url), "https://h.example/a/b");
    }

    #[test]
    fn missing_or_invalid_ca_files_are_reported() {
        let missing = ClientOptions::default().with_ca_file(Some("/nonexistent/ca.pem"));
        assert!(build_client(&missing).is_err());
        let dir = tempfile::tempdir().expect("tempdir");
        let bad = dir.path().join("ca.pem");
        std::fs::write(&bad, "not a pem").expect("write");
        let options = ClientOptions::default().with_ca_file(bad.to_str());
        assert!(build_client(&options).is_err());
    }

    #[test]
    fn default_client_builds() {
        assert!(
            build_client(&ClientOptions::default().with_cookies().with_user_agent("x")).is_ok()
        );
    }
}
