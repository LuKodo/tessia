//! [`ObjectStore`] over the Cassandra storage API.
//!
//! Contract (`<base>/api/v1/buckets/<bucket>/files`):
//!
//! | operation | request | success | missing |
//! |---|---|---|---|
//! | put | `POST .../files/upload` multipart (`file`, form field `name`) | 201 JSON | |
//! | get | `GET .../files/<name>/download` | 200 body | 404 `{"error"}` |
//! | delete | `DELETE .../files/<name>` | 200 | 404 |
//! | list | `GET .../files?limit=N&start=S` | `{items:[..], next_start}` | |
//!
//! There is no HEAD/exists endpoint (`/files/search` is unreliable), so
//! [`exists`](ObjectStore::exists) is a download whose body is dropped. The
//! name is a flat object name, percent-encoded as one path segment. A
//! same-name upload overwrites, which makes uploads safe to retry. Every
//! request carries the bearer token, which is never logged.

use std::fmt;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response, multipart};
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;

use super::{ListPage, ObjectInfo, ObjectStore, StorageError, StorageResult};
use crate::config::Settings;
use crate::http::{ClientOptions, RetryEvent, RetryPolicy, build_client, send_with_retry};
use crate::util::fsx;

/// Cassandra-backed bucket.
pub struct CassandraStore {
    client: Client,
    base: String,
    bucket: String,
    api_key: String,
    retry: RetryPolicy,
}

impl fmt::Debug for CassandraStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CassandraStore")
            .field("base", &self.base)
            .field("bucket", &self.bucket)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<String>,
}

#[derive(Deserialize)]
struct ListBody {
    #[serde(default)]
    items: Vec<ItemBody>,
    #[serde(default)]
    next_start: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ItemBody {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    updated_at: Option<String>,
}

impl CassandraStore {
    /// Builds a store against `base_url` / `bucket`. Retries use the default
    /// [`RetryPolicy`] (3 attempts) and log each retry without the URL query.
    pub fn new(base_url: &str, bucket: &str, api_key: &str) -> anyhow::Result<Self> {
        let client = build_client(&ClientOptions::default())?;
        Ok(Self::with_client(client, base_url, bucket, api_key))
    }

    /// Like [`CassandraStore::new`] with a caller-provided client.
    pub fn with_client(client: Client, base_url: &str, bucket: &str, api_key: &str) -> Self {
        let on_retry: Arc<dyn Fn(&RetryEvent) + Send + Sync> = Arc::new(|e| {
            tracing::warn!(
                host = %e.host, path = %e.path, attempt = e.attempt, max = e.max_attempts,
                status = e.status, error = %e.error, delay_ms = e.delay.as_millis() as u64,
                "object store request failed, retrying"
            );
        });
        Self {
            client,
            base: base_url.trim_end_matches('/').to_owned(),
            bucket: bucket.to_owned(),
            api_key: api_key.to_owned(),
            retry: RetryPolicy {
                on_retry: Some(on_retry),
                ..RetryPolicy::default()
            },
        }
    }

    /// From `cassandra.base_url`, `cassandra.bucket` and `cassandra.api_key`.
    pub fn from_settings(settings: &Settings) -> anyhow::Result<Self> {
        settings.require(&[
            "cassandra.base_url",
            "cassandra.bucket",
            "cassandra.api_key",
        ])?;
        let get = |key: &str| settings.get(key).unwrap_or_default().to_owned();
        Self::new(
            &get("cassandra.base_url"),
            &get("cassandra.bucket"),
            &get("cassandra.api_key"),
        )
    }

    /// Replaces the retry policy (tests use an instant one).
    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// `<base>/api/v1/buckets/<bucket>/files` followed by `segments`, each
    /// percent-encoded as one path segment.
    fn url(&self, segments: &[&str]) -> StorageResult<reqwest::Url> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| StorageError::Failed("cassandra.base_url inválida".into()))?;
        url.path_segments_mut()
            .map_err(|_| StorageError::Failed("cassandra.base_url inválida".into()))?
            .pop_if_empty()
            .extend(["api", "v1", "buckets", self.bucket.as_str(), "files"])
            .extend(segments);
        Ok(url)
    }

    fn send(&self, build: impl FnMut() -> RequestBuilder) -> StorageResult<Response> {
        send_with_retry(&self.retry, None, build)
            .map_err(|err| StorageError::Failed(err.to_string()))
    }

    fn authed(&self, request: RequestBuilder) -> RequestBuilder {
        request.bearer_auth(&self.api_key)
    }

    /// Maps a non-success response to an error (404 -> NotFound for `key`).
    fn failure(&self, response: Response, key: &str) -> StorageError {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        let message = serde_json::from_str::<ErrorBody>(&body)
            .ok()
            .and_then(|b| b.error)
            .unwrap_or_else(|| body.chars().take(200).collect::<String>());
        if status == StatusCode::NOT_FOUND {
            return StorageError::NotFound(key.to_owned());
        }
        StorageError::Failed(format!("HTTP {}: {message}", status.as_u16()))
    }

    fn upload(
        &self,
        key: &str,
        part: impl Fn() -> StorageResult<multipart::Part>,
    ) -> StorageResult<()> {
        let url = self.url(&["upload"])?;
        // Surface an unreadable file or a bad content type before any request.
        drop(part()?);
        // A same-name upload overwrites, so repeating a POST is safe.
        let policy = RetryPolicy {
            retry_non_idempotent: true,
            ..self.retry.clone()
        };
        let response = send_with_retry(&policy, None, || {
            // A new part per attempt: the body must be re-sendable.
            let form = match part() {
                Ok(part) => multipart::Form::new().part("file", part),
                Err(_) => multipart::Form::new(),
            };
            self.authed(self.client.post(url.clone()))
                .multipart(form.text("name", key.to_owned()))
        })
        .map_err(|err| StorageError::Failed(err.to_string()))?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(self.failure(response, key))
    }

    fn download(&self, key: &str) -> StorageResult<Response> {
        let url = self.url(&[key, "download"])?;
        let response = self.send(|| self.authed(self.client.get(url.clone())))?;
        if response.status().is_success() {
            return Ok(response);
        }
        Err(self.failure(response, key))
    }
}

fn file_part(path: &Path, key: &str, content_type: &str) -> StorageResult<multipart::Part> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    multipart::Part::reader_with_length(file, len)
        .file_name(key.to_owned())
        .mime_str(content_type)
        .map_err(|err| StorageError::Failed(format!("content-type inválido: {err}")))
}

impl ObjectStore for CassandraStore {
    fn put_bytes(&self, key: &str, data: &[u8], content_type: &str) -> StorageResult<()> {
        self.upload(key, || {
            multipart::Part::bytes(data.to_vec())
                .file_name(key.to_owned())
                .mime_str(content_type)
                .map_err(|err| StorageError::Failed(format!("content-type inválido: {err}")))
        })
    }

    fn put_file(&self, key: &str, path: &Path, content_type: &str) -> StorageResult<()> {
        self.upload(key, || file_part(path, key, content_type))
    }

    fn get_bytes(&self, key: &str) -> StorageResult<Vec<u8>> {
        let response = self.download(key)?;
        response
            .bytes()
            .map(|b| b.to_vec())
            .map_err(|err| StorageError::Failed(err.without_url().to_string()))
    }

    fn get_to_file(&self, key: &str, dest: &Path) -> StorageResult<u64> {
        let mut response = self.download(key)?;
        if let Some(parent) = dest.parent() {
            fsx::ensure_dir(parent).map_err(|err| StorageError::Failed(format!("{err:#}")))?;
        }
        let tmp = dest.with_extension(format!("part-{}", uuid::Uuid::new_v4().simple()));
        let result = (|| -> StorageResult<u64> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            let size = response
                .copy_to(&mut file)
                .map_err(|err| StorageError::Failed(err.without_url().to_string()))?;
            file.sync_all()?;
            std::fs::rename(&tmp, dest)?;
            Ok(size)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    fn exists(&self, key: &str) -> StorageResult<bool> {
        match self.download(key) {
            Ok(response) => {
                drop(response); // the body is not read
                Ok(true)
            }
            Err(StorageError::NotFound(_)) => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn delete(&self, key: &str) -> StorageResult<bool> {
        let url = self.url(&[key])?;
        let response = self.send(|| self.authed(self.client.delete(url.clone())))?;
        if response.status().is_success() {
            return Ok(true);
        }
        match self.failure(response, key) {
            StorageError::NotFound(_) => Ok(false),
            other => Err(other),
        }
    }

    fn list(&self, start: Option<&str>, limit: usize) -> StorageResult<ListPage> {
        let mut url = self.url(&[])?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &limit.max(1).to_string());
            if let Some(start) = start {
                query.append_pair("start", start);
            }
        }
        let response = self.send(|| {
            self.authed(self.client.get(url.clone()))
                .header(CONTENT_TYPE, "application/json")
        })?;
        if !response.status().is_success() {
            return Err(self.failure(response, ""));
        }
        let body: ListBody = response.json().map_err(|err| {
            StorageError::Failed(format!(
                "respuesta de listado inválida: {}",
                err.without_url()
            ))
        })?;
        let next_start = match body.next_start {
            Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        Ok(ListPage {
            items: body
                .items
                .into_iter()
                .map(|i| ObjectInfo {
                    name: i.name,
                    size: i.size,
                    content_type: i.content_type,
                    updated_at: i.updated_at,
                })
                .collect(),
            next_start,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::testing::{MockResponse, MockServer};

    const BASE: &str = "/api/v1/buckets/b1/files";

    fn store(server: &MockServer) -> CassandraStore {
        CassandraStore::new(server.url(), "b1", "SECRET-KEY")
            .expect("store")
            .with_retry(RetryPolicy::instant(3))
    }

    #[test]
    fn put_bytes_posts_a_multipart_with_name_and_bearer() {
        let server = MockServer::start(|_| {
            MockResponse::json(
                201,
                r#"{"message":"ok","file":{"name":"C1__XML__a.xml","size":3,"content_type":"application/xml"},"view_url":"/x"}"#,
            )
        });
        store(&server)
            .put_bytes("C1__XML__a.xml", b"<a/>", "application/xml")
            .expect("put");
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        let r = &requests[0];
        assert_eq!(
            (r.method.as_str(), r.path.as_str()),
            ("POST", &*format!("{BASE}/upload"))
        );
        assert_eq!(r.header("authorization"), Some("Bearer SECRET-KEY"));
        assert!(
            r.header("content-type")
                .unwrap_or_default()
                .starts_with("multipart/form-data")
        );
        let body = r.body_text();
        assert!(
            body.contains("name=\"file\"")
                && body.contains("filename=\"C1__XML__a.xml\"")
                && body.contains("<a/>")
        );
        assert!(body.contains("name=\"name\"") && body.contains("C1__XML__a.xml"));
    }

    #[test]
    fn uploads_are_retried_on_server_errors_and_resend_the_body() {
        let server = MockServer::sequence(vec![
            MockResponse::text(503, "busy"),
            MockResponse::json(201, "{}"),
        ]);
        store(&server)
            .put_bytes("k", b"payload", "text/plain")
            .expect("put");
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r.body_text().contains("payload")));
    }

    #[test]
    fn put_file_streams_the_file() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("f.pdf");
        std::fs::write(&path, b"%PDF-data").expect("write");
        let server = MockServer::start(|_| MockResponse::json(201, "{}"));
        store(&server)
            .put_file("C1__f.pdf", &path, "application/pdf")
            .expect("put");
        assert!(server.requests()[0].body_text().contains("%PDF-data"));
    }

    #[test]
    fn upload_failures_carry_the_service_message_but_never_the_token() {
        let server = MockServer::start(|_| {
            MockResponse::json(400, r#"{"error":"Nombre de archivo vacío"}"#)
        });
        let err = store(&server)
            .put_bytes("k", b"x", "text/plain")
            .expect_err("rejected");
        let text = err.to_string();
        assert!(
            text.contains("400") && text.contains("Nombre de archivo vacío"),
            "{text}"
        );
        assert!(!text.contains("SECRET-KEY"));
        assert!(!format!("{:?}", store(&server)).contains("SECRET-KEY"));
    }

    #[test]
    fn get_encodes_the_name_and_maps_404_to_not_found() {
        let server = MockServer::start(|req| {
            if req.path.contains("missing") {
                MockResponse::json(404, r#"{"error":"not found"}"#)
            } else {
                MockResponse::bytes(200, b"BYTES".to_vec())
            }
        });
        let s = store(&server);
        assert_eq!(s.get_bytes("C1__a b/ñ.xml").expect("get"), b"BYTES");
        assert_eq!(
            server.requests()[0].path,
            format!("{BASE}/C1__a%20b%2F%C3%B1.xml/download")
        );
        assert!(matches!(s.get_bytes("missing"), Err(StorageError::NotFound(k)) if k == "missing"));
    }

    #[test]
    fn exists_is_a_download_with_the_body_dropped() {
        let server = MockServer::start(|req| {
            if req.path.contains("nope") {
                MockResponse::json(404, r#"{"error":"x"}"#)
            } else {
                MockResponse::bytes(200, vec![0; 1024])
            }
        });
        let s = store(&server);
        assert!(s.exists("there").expect("exists"));
        assert!(!s.exists("nope").expect("missing"));
        let broken = MockServer::start(|_| MockResponse::json(403, r#"{"error":"forbidden"}"#));
        assert!(store(&broken).exists("x").is_err());
    }

    #[test]
    fn get_to_file_writes_atomically() {
        let server = MockServer::start(|_| MockResponse::bytes(200, b"0123456789".to_vec()));
        let dir = tempfile::tempdir().expect("dir");
        let dest = dir.path().join("a/b.bin");
        assert_eq!(store(&server).get_to_file("k", &dest).expect("get"), 10);
        assert_eq!(std::fs::read(&dest).expect("read"), b"0123456789");
        assert_eq!(
            std::fs::read_dir(dest.parent().expect("parent"))
                .expect("ls")
                .count(),
            1
        );
    }

    #[test]
    fn delete_reports_whether_the_object_existed() {
        let server = MockServer::start(|req| {
            if req.path.contains("gone") {
                MockResponse::json(404, r#"{"error":"x"}"#)
            } else {
                MockResponse::json(200, r#"{"message":"Archivo eliminado"}"#)
            }
        });
        let s = store(&server);
        assert!(s.delete("k").expect("deleted"));
        assert!(!s.delete("gone").expect("absent"));
        let r = &server.requests()[0];
        assert_eq!(
            (r.method.as_str(), r.header("authorization")),
            ("DELETE", Some("Bearer SECRET-KEY"))
        );
    }

    #[test]
    fn list_sends_limit_and_start_and_parses_the_page() {
        let server = MockServer::sequence(vec![
            MockResponse::json(
                200,
                r#"{"items":[{"name":"C1__a","size":3,"content_type":"text/plain","updated_at":"2026-10-22T10:00:00Z"}],"next_start":"C1__b"}"#,
            ),
            MockResponse::json(
                200,
                r#"{"items":[{"name":"C1__b","size":1,"content_type":"text/plain"}],"next_start":null}"#,
            ),
        ]);
        let s = store(&server);
        let first = s.list(None, 1).expect("page");
        assert_eq!(
            first.items[0],
            ObjectInfo {
                name: "C1__a".into(),
                size: 3,
                content_type: "text/plain".into(),
                updated_at: Some("2026-10-22T10:00:00Z".into())
            }
        );
        assert_eq!(first.next_start.as_deref(), Some("C1__b"));
        let second = s.list(first.next_start.as_deref(), 1).expect("page");
        assert_eq!(second.next_start, None);
        let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
        assert_eq!(
            paths,
            [
                format!("{BASE}?limit=1"),
                format!("{BASE}?limit=1&start=C1__b")
            ]
        );
    }

    #[test]
    fn list_prefix_walks_every_page() {
        let server = MockServer::sequence(vec![
            MockResponse::json(
                200,
                r#"{"items":[{"name":"C1__a","size":1,"content_type":""},{"name":"C2__a","size":1,"content_type":""}],"next_start":"x"}"#,
            ),
            MockResponse::json(
                200,
                r#"{"items":[{"name":"C1__b","size":1,"content_type":""}],"next_start":""}"#,
            ),
        ]);
        let names: Vec<String> = store(&server)
            .list_prefix("C1__")
            .expect("list")
            .into_iter()
            .map(|o| o.name)
            .collect();
        assert_eq!(names, ["C1__a", "C1__b"]);
    }

    #[test]
    fn unreachable_servers_fail_after_the_retries() {
        let s = CassandraStore::new("http://127.0.0.1:9", "b1", "k")
            .expect("store")
            .with_retry(RetryPolicy {
                max_attempts: 2,
                sleeper: Some(Arc::new(|_: Duration| {})),
                ..RetryPolicy::default()
            });
        assert!(s.get_bytes("x").is_err());
    }
}
