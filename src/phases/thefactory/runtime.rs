//! The external clients of the phase, built from the settings.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use super::client::{ClientConfig, TheFactoryClient};
use super::cpscan::{Acquirer, CpscanApi, CpscanFiles, CpscanHttp, CpscanTransport};
use crate::context::AppContext;
use crate::http::{ClientOptions, RetryPolicy, build_client};

/// Keys a run needs (TF-10; the MySQL DSN is gone).
pub const REQUIRED_KEYS: [&str; 5] = [
    "thefactory.base_url",
    "thefactory.token_empresa",
    "thefactory.token_password",
    "cpscan.base_url",
    "cpscan.bucket_url",
];

/// Request timeout of The Factory (TF-09); it starts after the limiter slot.
const THEFACTORY_TIMEOUT: Duration = Duration::from_secs(60);

/// The Factory client and the CPScan acquirer a run uses.
pub struct Runtime {
    pub thefactory: TheFactoryClient,
    pub cpscan: Acquirer,
}

impl Runtime {
    /// Builds both clients from `thefactory.*` / `cpscan.*`, sharing the
    /// process-wide limiters of the context. Fails naming the missing keys.
    pub fn from_settings(ctx: &AppContext) -> Result<Runtime> {
        let settings = &ctx.settings;
        settings.require(&REQUIRED_KEYS)?;
        let http = build_client(
            &ClientOptions::default()
                .with_timeout(THEFACTORY_TIMEOUT)
                .with_ca_file(settings.get("thefactory.ca_file")),
        )?;
        let thefactory = TheFactoryClient::new(ClientConfig {
            base_url: settings.get_or("thefactory.base_url", "").to_owned(),
            token_empresa: settings.get_or("thefactory.token_empresa", "").to_owned(),
            token_password: settings.get_or("thefactory.token_password", "").to_owned(),
            http,
            limiter: ctx.limiters.get("thefactory"),
            sleeper: None,
        })?;
        let transport = Arc::new(CpscanTransport::new(
            CpscanHttp::new(
                settings.get("cpscan.ca_file"),
                settings.get("cpscan.tls_server_name"),
            )?,
            ctx.limiters.get("cpscan"),
            RetryPolicy::default(),
        ));
        let api = CpscanApi::new(settings.get_or("cpscan.base_url", ""), transport.clone())?;
        let files = CpscanFiles::new(settings.get_or("cpscan.bucket_url", ""), transport)?;
        Ok(Runtime {
            thefactory,
            cpscan: Acquirer::new(api, files),
        })
    }
}
