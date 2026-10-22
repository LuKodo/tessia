//! The application context every phase receives.
//!
//! ```no_run
//! use tessia::{config::Settings, context::AppContext};
//!
//! # fn demo() -> anyhow::Result<()> {
//! let ctx = AppContext::from_settings(Settings::load(None)?)?; // connects and migrates
//! let wd = ctx.workdir("Convenio_21102026_21102026")?; // temp folder, removed on drop
//! let _claims = ctx.proceso();
//! # let _ = wd; Ok(()) }
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::Settings;
use crate::db::{DbPool, migrate};
use crate::http::Limiters;
use crate::nt::{NtCatalog, RedisNt};
use crate::storage::{Artifacts, CassandraStore, ObjectStore, Workdir};
use crate::stores::artefactos::ArtefactosStore;
use crate::stores::cargues::CarguesStore;
use crate::stores::derechos::DerechosCache;
use crate::stores::leases::LeaseStore;
use crate::stores::parametros::ParametrosStore;
use crate::stores::proceso::{ClaimOptions, ProcesoStore};
use crate::stores::radicacion::RadicacionStore;
use crate::stores::valor_xml::ValorXmlStore;

/// How long an `EN_PROGRESO` claim may go unfinished before another worker may
/// take it over (a crashed worker's claim). Longer than the slowest phase run
/// on one invoice.
pub const DEFAULT_CLAIM_TTL: Duration = Duration::from_secs(30 * 60);

/// Everything a phase needs. Cheap to clone (everything inside is shared).
#[derive(Clone)]
pub struct AppContext {
    pub settings: Settings,
    pub pool: DbPool,
    /// The bucket for cargue artifacts.
    pub objects: Arc<dyn ObjectStore>,
    /// The Nota Técnica collection.
    pub nt: Arc<dyn NtCatalog>,
    /// Parent of every [`Workdir`].
    pub workdir_root: PathBuf,
    /// Identity of this process in claims and leases: `host:pid:uuid`.
    pub owner: String,
    /// Process-wide AIMD limiters per external system.
    pub limiters: Arc<Limiters>,
    /// Keeps a test workdir root alive (removed when the last clone drops).
    _temp_root: Option<Arc<TempRoot>>,
}

impl std::fmt::Debug for AppContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppContext")
            .field("owner", &self.owner)
            .field("workdir_root", &self.workdir_root)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `host:pid:uuid`, unique per process run.
pub fn new_owner() -> String {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "desconocido".into());
    format!(
        "{host}:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    )
}

impl AppContext {
    /// The real context: connects to Postgres and applies the pending
    /// migrations (safe with several machines starting at once), then opens the
    /// Cassandra object store and the Redis NT (both connect lazily).
    pub fn from_settings(settings: Settings) -> Result<AppContext> {
        let url = settings.require_one("database.url")?;
        migrate::migrate_url(url).context("no se pudieron aplicar las migraciones")?;
        let pool = DbPool::from_settings(&settings)?;
        let objects: Arc<dyn ObjectStore> = Arc::new(CassandraStore::from_settings(&settings)?);
        let nt: Arc<dyn NtCatalog> = Arc::new(RedisNt::from_settings(&settings)?);
        let workdir_root = settings.workdir_root();
        Ok(AppContext {
            settings,
            pool,
            objects,
            nt,
            workdir_root,
            owner: new_owner(),
            limiters: Arc::new(Limiters::new()),
            _temp_root: None,
        })
    }

    /// A context for tests: the given database, an in-memory object store and
    /// NT, and a private temp workdir root removed when the context is dropped.
    /// Replace the fakes with [`with_objects`](Self::with_objects) /
    /// [`with_nt`](Self::with_nt).
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(db: &crate::db::test_support::TestDb) -> AppContext {
        let root =
            std::env::temp_dir().join(format!("tessia-test-{}", uuid::Uuid::new_v4().simple()));
        let _ = std::fs::create_dir_all(&root);
        AppContext {
            settings: Settings::default().with("workdir.root", &root.to_string_lossy()),
            pool: db.pool(),
            objects: Arc::new(crate::storage::MemoryStore::new()),
            nt: Arc::new(crate::nt::MemoryNt::new()),
            workdir_root: root.clone(),
            owner: new_owner(),
            limiters: Arc::new(Limiters::new()),
            _temp_root: Some(Arc::new(TempRoot(root))),
        }
    }

    #[must_use]
    pub fn with_objects(mut self, objects: Arc<dyn ObjectStore>) -> Self {
        self.objects = objects;
        self
    }

    #[must_use]
    pub fn with_nt(mut self, nt: Arc<dyn NtCatalog>) -> Self {
        self.nt = nt;
        self
    }

    #[must_use]
    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// Another worker identity (tests that simulate a second machine).
    #[must_use]
    pub fn with_owner(mut self, owner: &str) -> Self {
        self.owner = owner.to_owned();
        self
    }

    /// A new temp workdir for the cargue; it deletes itself when dropped.
    pub fn workdir(&self, cargue_id: &str) -> Result<Workdir> {
        Workdir::create(&self.workdir_root, cargue_id)
    }

    /// Publish/fetch of cargue artifacts.
    pub fn artifacts(&self) -> Artifacts {
        Artifacts::new(self.objects.clone(), self.artefactos())
    }

    /// Claim options of this process for the cargue, with the default TTL.
    pub fn claim_options(&self, cargue_id: &str) -> ClaimOptions {
        ClaimOptions::new(&self.owner, Some(cargue_id), DEFAULT_CLAIM_TTL)
    }

    pub fn cargues(&self) -> CarguesStore {
        CarguesStore::new(self.pool.clone())
    }

    pub fn artefactos(&self) -> ArtefactosStore {
        ArtefactosStore::new(self.pool.clone())
    }

    pub fn proceso(&self) -> ProcesoStore {
        ProcesoStore::new(self.pool.clone())
    }

    pub fn radicacion(&self) -> RadicacionStore {
        RadicacionStore::new(self.pool.clone())
    }

    pub fn derechos_cache(&self) -> DerechosCache {
        DerechosCache::new(self.pool.clone())
    }

    pub fn parametros(&self) -> ParametrosStore {
        ParametrosStore::new(self.pool.clone())
    }

    pub fn valor_xml(&self) -> ValorXmlStore {
        ValorXmlStore::new(self.pool.clone())
    }

    pub fn leases(&self) -> LeaseStore {
        LeaseStore::new(self.pool.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;

    #[test]
    fn owners_identify_host_pid_and_run() {
        let a = new_owner();
        let b = new_owner();
        assert_ne!(a, b);
        assert_eq!(a.split(':').count(), 3);
        assert!(a.contains(&std::process::id().to_string()));
    }

    #[test]
    fn test_context_wires_the_fakes_and_cleans_its_workdir_root() {
        let db = TestDb::new();
        let ctx = AppContext::for_tests(&db);
        let root = ctx.workdir_root.clone();
        assert!(root.exists());
        let wd = ctx.workdir("C1").expect("workdir");
        wd.write("XML/a.xml", b"<a/>").expect("write");
        ctx.artifacts().publish_workdir(&wd, None).expect("publish");
        assert_eq!(ctx.artifacts().list("C1").expect("list").len(), 1);
        assert_eq!(ctx.nt.current_version().expect("nt"), None);
        let clone = ctx.clone();
        drop(ctx);
        assert!(root.exists(), "clones keep the root alive");
        drop(wd);
        drop(clone);
        assert!(!root.exists());
    }
}
