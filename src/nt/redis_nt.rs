//! [`NtCatalog`] over the Redis collection (read-only, synchronous).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use redis::Commands;

use super::row::base_id;
use super::{NtCatalog, NtError, NtResult, NtRow};
use crate::config::Settings;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Redis-backed Nota Técnica. Cheap to share: connections are pooled
/// internally and the field index of each version is cached on first use.
pub struct RedisNt {
    client: redis::Client,
    prefix: String,
    idle: Mutex<Vec<redis::Connection>>,
    /// version -> base id -> hash fields (`<id>` and `<id>#N`).
    fields: Mutex<HashMap<String, HashMap<String, Vec<String>>>>,
}

impl std::fmt::Debug for RedisNt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisNt")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl RedisNt {
    /// Connects lazily to `url` (`redis://host:6379`); keys start with `prefix`.
    pub fn new(url: &str, prefix: &str) -> NtResult<Self> {
        Ok(Self {
            client: redis::Client::open(url)?,
            prefix: prefix.to_owned(),
            idle: Mutex::new(Vec::new()),
            fields: Mutex::new(HashMap::new()),
        })
    }

    /// From `redis.url` and `redis.nt_prefix`.
    pub fn from_settings(settings: &Settings) -> anyhow::Result<Self> {
        let url = settings.require_one("redis.url")?;
        Ok(Self::new(url, settings.get_or("redis.nt_prefix", "nt:"))?)
    }

    fn key(&self, parts: &[&str]) -> String {
        format!("{}{}", self.prefix, parts.join(":"))
    }

    /// Runs `op` on a pooled connection; a connection that failed is dropped
    /// and the operation is retried once on a fresh one.
    fn with_conn<T>(
        &self,
        mut op: impl FnMut(&mut redis::Connection) -> redis::RedisResult<T>,
    ) -> NtResult<T> {
        let mut last = None;
        for _ in 0..2 {
            let reused = self.idle.lock().unwrap_or_else(|p| p.into_inner()).pop();
            let mut conn = match reused {
                Some(conn) => conn,
                None => {
                    let conn = self.client.get_connection_with_timeout(CONNECT_TIMEOUT)?;
                    conn.set_read_timeout(Some(IO_TIMEOUT))?;
                    conn.set_write_timeout(Some(IO_TIMEOUT))?;
                    conn
                }
            };
            match op(&mut conn) {
                Ok(value) => {
                    self.idle
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(conn);
                    return Ok(value);
                }
                Err(err) if err.is_io_error() || err.is_connection_dropped() => last = Some(err),
                Err(err) => return Err(err.into()),
            }
        }
        Err(last.map_or_else(|| NtError::Redis("sin conexión".into()), NtError::from))
    }

    /// base id -> hash fields of the version, built once with `HKEYS`.
    fn field_index(&self, version: &str) -> NtResult<HashMap<String, Vec<String>>> {
        if let Some(index) = self
            .fields
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(version)
        {
            return Ok(index.clone());
        }
        let key = self.key(&[version, "filas"]);
        let names: Vec<String> = self.with_conn(|c| c.hkeys(&key))?;
        let mut index: HashMap<String, Vec<String>> = HashMap::new();
        for name in names {
            index
                .entry(base_id(&name).to_owned())
                .or_default()
                .push(name);
        }
        self.fields
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(version.to_owned(), index.clone());
        Ok(index)
    }

    fn parse(version: &str, id: &str, json: &str) -> NtResult<NtRow> {
        NtRow::from_json(id, json).map_err(|err| NtError::BadRow {
            version: version.to_owned(),
            id: id.to_owned(),
            message: err.to_string(),
        })
    }
}

impl NtCatalog for RedisNt {
    fn current_version(&self) -> NtResult<Option<String>> {
        let key = self.key(&["actual"]);
        if let Some(current) = self
            .with_conn(|c| c.get::<_, Option<String>>(&key))?
            .filter(|v| !v.is_empty())
        {
            return Ok(Some(current));
        }
        // No pointer: the newest known version.
        let versions_key = self.key(&["versiones"]);
        Ok(self
            .with_conn(|c| c.zrevrange::<_, Vec<String>>(&versions_key, 0, 0))?
            .into_iter()
            .next())
    }

    fn versions(&self) -> NtResult<Vec<String>> {
        let key = self.key(&["versiones"]);
        self.with_conn(|c| c.zrange(&key, 0, -1))
    }

    fn meta(&self, version: &str) -> NtResult<BTreeMap<String, String>> {
        let key = self.key(&[version, "meta"]);
        let meta: HashMap<String, String> = self.with_conn(|c| c.hgetall(&key))?;
        Ok(meta.into_iter().collect())
    }

    fn lookup(&self, version: &str, field: &str, value: &str) -> NtResult<Vec<NtRow>> {
        let set_key = self.key(&[version, "idx", field, value]);
        let members: Vec<String> = self.with_conn(|c| c.smembers(&set_key))?;
        if members.is_empty() {
            return Ok(Vec::new());
        }
        let index = self.field_index(version)?;
        let mut wanted: Vec<String> = Vec::new();
        for member in &members {
            match index.get(base_id(member)) {
                Some(fields) => wanted.extend(fields.iter().cloned()),
                None => wanted.push(member.clone()),
            }
        }
        wanted.sort();
        wanted.dedup();
        let rows_key = self.key(&[version, "filas"]);
        let mut rows = Vec::with_capacity(wanted.len());
        for chunk in wanted.chunks(500) {
            let values: Vec<Option<String>> =
                self.with_conn(|c| redis::cmd("HMGET").arg(&rows_key).arg(chunk).query(c))?;
            for (id, json) in chunk.iter().zip(values) {
                if let Some(json) = json {
                    rows.push(Self::parse(version, id, &json)?);
                }
            }
        }
        Ok(rows)
    }

    fn row(&self, version: &str, id: &str) -> NtResult<Option<NtRow>> {
        let key = self.key(&[version, "filas"]);
        let json: Option<String> = self.with_conn(|c| c.hget(&key, id))?;
        json.map(|j| Self::parse(version, id, &j)).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redis_url() -> String {
        std::env::var("TESSIA_TEST_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:56379".to_owned())
    }

    /// A random key prefix with the data of the sample NT, deleted on drop.
    struct Seeded {
        client: redis::Client,
        prefix: String,
    }

    impl Seeded {
        fn new() -> Seeded {
            let client = redis::Client::open(redis_url()).expect("redis url");
            let mut conn = match client.get_connection_with_timeout(Duration::from_secs(5)) {
                Ok(conn) => conn,
                Err(err) => panic!(
                    "no hay Redis de prueba en {} (TESSIA_TEST_REDIS_URL): {err}",
                    redis_url()
                ),
            };
            let prefix = format!("nt_test_{}:", uuid::Uuid::new_v4().simple());
            let p = prefix.clone();
            let _: () = redis::pipe()
                .set(format!("{p}actual"), "v2")
                .zadd(format!("{p}versiones"), "v1", 1)
                .zadd(format!("{p}versiones"), "v2", 2)
                .hset(format!("{p}v2:meta"), "filas", "3")
                .hset(format!("{p}v2:meta"), "fuente", "prueba")
                .hset(format!("{p}v2:filas"), "10", r#"{"cum_facturar":"A-1","cod_auf":"X1","estado":"VIGENTE","precio_final":12.5}"#)
                .hset(format!("{p}v2:filas"), "10#1", r#"{"cum_facturar":"A-1","cod_auf":"X1","estado":"HISTORICO"}"#)
                .hset(format!("{p}v2:filas"), "20", r#"{"cum_facturar":"B-2","cod_auf":"X2","estado":"VIGENTE","extra_col":7}"#)
                .sadd(format!("{p}v2:idx:cum_facturar:A-1"), "10")
                .sadd(format!("{p}v2:idx:cum_facturar:B-2"), "20")
                .sadd(format!("{p}v2:idx:estado:VIGENTE"), vec!["10", "20"])
                .sadd(format!("{p}v2:idx:cod_auf:X1"), "10#1")
                .query(&mut conn)
                .expect("seed");
            Seeded { client, prefix }
        }

        fn nt(&self) -> RedisNt {
            RedisNt::new(&redis_url(), &self.prefix).expect("nt")
        }
    }

    impl Drop for Seeded {
        fn drop(&mut self) {
            if let Ok(mut conn) = self.client.get_connection() {
                let keys: Vec<String> = conn
                    .scan_match::<_, String>(format!("{}*", self.prefix))
                    .map(|it| it.filter_map(Result::ok).collect())
                    .unwrap_or_default();
                if !keys.is_empty() {
                    let _: redis::RedisResult<()> = conn.del(keys);
                }
            }
        }
    }

    #[test]
    fn reads_version_pointer_versions_and_meta() {
        let seeded = Seeded::new();
        let nt = seeded.nt();
        assert_eq!(
            nt.current_version().expect("current").as_deref(),
            Some("v2")
        );
        assert_eq!(nt.versions().expect("versions"), ["v1", "v2"]);
        let meta = nt.meta("v2").expect("meta");
        assert_eq!(
            (meta["filas"].as_str(), meta["fuente"].as_str()),
            ("3", "prueba")
        );
        assert!(nt.meta("v9").expect("unknown").is_empty());
    }

    #[test]
    fn current_version_falls_back_to_the_newest_known() {
        let seeded = Seeded::new();
        let mut conn = seeded.client.get_connection().expect("conn");
        let _: () = conn.del(format!("{}actual", seeded.prefix)).expect("del");
        assert_eq!(
            seeded.nt().current_version().expect("current").as_deref(),
            Some("v2")
        );
    }

    #[test]
    fn lookup_resolves_ids_through_the_sets_and_brings_variants() {
        let seeded = Seeded::new();
        let nt = seeded.nt();
        let rows = nt.lookup("v2", "cum_facturar", "A-1").expect("lookup");
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["10", "10#1"]
        );
        assert_eq!(rows[0].precio_final, Some("12.5".parse().expect("dec")));
        // The set lists a variant id: the whole family still comes back.
        let by_variant = nt.lookup("v2", "cod_auf", "X1").expect("lookup");
        assert_eq!(by_variant.len(), 2);
        let vigentes = nt.lookup("v2", "estado", "VIGENTE").expect("lookup");
        assert_eq!(
            vigentes.iter().map(|r| r.base_id()).collect::<Vec<_>>(),
            ["10", "10", "20"]
        );
        assert_eq!(
            vigentes[2].extra.get("extra_col"),
            Some(&serde_json::json!(7))
        );
        assert!(
            nt.lookup("v2", "cum_facturar", "NOPE")
                .expect("none")
                .is_empty()
        );
    }

    #[test]
    fn row_reads_a_single_hash_field() {
        let seeded = Seeded::new();
        let nt = seeded.nt();
        assert_eq!(
            nt.row("v2", "10#1")
                .expect("row")
                .and_then(|r| r.estado)
                .as_deref(),
            Some("HISTORICO")
        );
        assert!(nt.row("v2", "99").expect("row").is_none());
    }

    #[test]
    fn invalid_json_rows_are_reported_with_their_id() {
        let seeded = Seeded::new();
        let mut conn = seeded.client.get_connection().expect("conn");
        let _: () = conn
            .hset(format!("{}v2:filas", seeded.prefix), "30", "not json")
            .expect("hset");
        let err = seeded.nt().row("v2", "30").expect_err("bad row");
        assert!(err.to_string().contains("30"), "{err}");
    }

    #[test]
    fn unreachable_redis_is_an_error_not_a_panic() {
        let nt = RedisNt::new("redis://127.0.0.1:1", "nt:").expect("client");
        assert!(nt.current_version().is_err());
    }
}
