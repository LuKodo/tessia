//! `GET /api/health`: reachability of the adjacent services (Go:
//! `internal/dashboard/health.go`).
//!
//! These are TCP-reachability checks, not authenticated calls: they answer "is
//! the host up" without spending a login on the real service. Every check runs
//! concurrently and is capped at [`HEALTH_TIMEOUT`].
//!
//! Row order: Medicar, SISPRO, `ZonaSer · radicación`,
//! `PortalMutualser · autorizaciones`, the database, then the two rows that
//! only exist in this port: Redis (nota técnica) and the object store.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use diesel::RunQueryDsl;
use serde::Serialize;

use crate::api::{ApiResult, ApiState, request::Request, request::Response};
use crate::config::Settings;
use crate::db::DbPool;

/// Cap of one check (Go `healthTimeout`).
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);

/// Name of the database row. The UI special-cases this exact name to word its
/// detail line (`ping` instead of `alcance TCP`, `health-model.ts`), so it keeps
/// the Go name even though the database is Postgres. Renaming it to
/// `PostgreSQL` is this constant plus `s.nombre === "MySQL"` in that file.
pub const DATABASE_CHECK_NAME: &str = "MySQL";

pub const REDIS_CHECK_NAME: &str = "Redis · nota técnica";
pub const OBJECTS_CHECK_NAME: &str = "Almacenamiento · cargues";

const NOT_CONFIGURED: &str = "no configurado";
const INVALID_URL: &str = "URL invalida";
const SOME_URLS_DOWN: &str = "una o más URL no disponibles";

/// One row of the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Service {
    pub nombre: String,
    pub disponible: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detalle: String,
    pub latencia_ms: u64,
}

impl Service {
    fn down(name: &str, detail: &str) -> Self {
        Self {
            nombre: name.to_owned(),
            disponible: false,
            detalle: detail.to_owned(),
            latencia_ms: 0,
        }
    }
}

pub fn get(state: &ApiState, _req: &Request) -> ApiResult {
    Ok(Response::json(&check_all(state)))
}

/// Every row, in the documented order.
pub fn check_all(state: &ApiState) -> Vec<Service> {
    let settings = &state.ctx.settings;
    let groups: [(&str, Vec<String>); 4] = [
        ("Medicar", urls(settings, &["medicar.base_url"])),
        ("SISPRO", urls(settings, &["minsalud.base_url"])),
        (
            "ZonaSer · radicación",
            urls(
                settings,
                &["zonaser.login_url", "zonaser.benavente_base_url"],
            ),
        ),
        (
            "PortalMutualser · autorizaciones",
            urls(settings, &["portalmutualser.base_url"]),
        ),
    ];
    let mut handles: Vec<thread::JoinHandle<Service>> = groups
        .into_iter()
        .map(|(name, urls)| thread::spawn(move || check_url_group(name, &urls, HEALTH_TIMEOUT)))
        .collect();

    let pool = state.pool.clone();
    handles.push(thread::spawn(move || check_database(&pool, HEALTH_TIMEOUT)));
    let nt = state.ctx.nt.clone();
    handles.push(thread::spawn(move || {
        timed(REDIS_CHECK_NAME, HEALTH_TIMEOUT, move || {
            nt.current_version()
                .map(|_| ())
                .map_err(|_| "no se pudo consultar Redis".to_owned())
        })
    }));
    let objects = state.ctx.objects.clone();
    handles.push(thread::spawn(move || {
        timed(OBJECTS_CHECK_NAME, HEALTH_TIMEOUT, move || {
            objects
                .exists("health-probe")
                .map(|_| ())
                .map_err(|_| "no se pudo consultar el almacenamiento".to_owned())
        })
    }));

    let names = [
        "Medicar",
        "SISPRO",
        "ZonaSer · radicación",
        "PortalMutualser · autorizaciones",
        DATABASE_CHECK_NAME,
        REDIS_CHECK_NAME,
        OBJECTS_CHECK_NAME,
    ];
    handles
        .into_iter()
        .zip(names)
        .map(|(handle, name)| {
            handle
                .join()
                .unwrap_or_else(|_| Service::down(name, "error interno"))
        })
        .collect()
}

fn urls(settings: &Settings, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|key| settings.get(key).unwrap_or("").trim().to_owned())
        .collect()
}

/// Runs `probe` on its own thread and gives up after `timeout` (a hung socket
/// or driver must not hold the answer). The thread is left to finish alone.
fn timed(
    name: &str,
    timeout: Duration,
    probe: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Service {
    let (tx, rx) = mpsc::channel();
    let start = Instant::now();
    thread::spawn(move || {
        let _ = tx.send(probe());
    });
    let outcome = rx
        .recv_timeout(timeout)
        .unwrap_or_else(|_| Err("i/o timeout".to_owned()));
    let latencia_ms = start.elapsed().as_millis() as u64;
    match outcome {
        Ok(()) => Service {
            nombre: name.to_owned(),
            disponible: true,
            detalle: String::new(),
            latencia_ms,
        },
        Err(detail) => Service {
            nombre: name.to_owned(),
            disponible: false,
            detalle: detail,
            latencia_ms,
        },
    }
}

/// `SELECT 1` on one connection of the API pool.
fn check_database(pool: &DbPool, timeout: Duration) -> Service {
    let pool = pool.clone();
    timed(DATABASE_CHECK_NAME, timeout, move || {
        let mut conn = pool
            .get()
            .map_err(|_| "no se pudo conectar a la base de datos".to_owned())?;
        diesel::sql_query("SELECT 1")
            .execute(&mut conn)
            .map(|_| ())
            .map_err(|_| "no se pudo consultar la base de datos".to_owned())
    })
}

/// One URL: TCP reachability of its host and port.
fn check_url(name: &str, raw: &str, timeout: Duration) -> Service {
    let raw = raw.trim();
    if raw.is_empty() {
        return Service::down(name, NOT_CONFIGURED);
    }
    let Some((host, port)) = host_and_port(raw) else {
        return Service::down(name, INVALID_URL);
    };
    timed(name, timeout, move || {
        let addrs = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|err| err.to_string())?;
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(_) => return Ok(()),
                Err(err) => last = err,
            }
        }
        Err(if last.kind() == io::ErrorKind::TimedOut {
            "i/o timeout".to_owned()
        } else {
            last.to_string()
        })
    })
}

/// Host and port of `raw`; the port defaults to 80 for `http` and 443 otherwise
/// (Go). `None` when `raw` is not a URL with a host.
fn host_and_port(raw: &str) -> Option<(String, u16)> {
    let url = url::Url::parse(raw).ok()?;
    let host = match url.host()? {
        url::Host::Domain(domain) => domain.to_owned(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => ip.to_string(),
    };
    let port = url
        .port()
        .unwrap_or(if url.scheme() == "http" { 80 } else { 443 });
    Some((host, port))
}

/// Several URLs of one service: all must answer; the slowest latency is kept.
fn check_url_group(name: &str, urls: &[String], timeout: Duration) -> Service {
    match urls {
        [] => Service::down(name, NOT_CONFIGURED),
        [single] => check_url(name, single, timeout),
        many => {
            let handles: Vec<_> = many
                .iter()
                .map(|raw| {
                    let (name, raw) = (name.to_owned(), raw.clone());
                    thread::spawn(move || check_url(&name, &raw, timeout))
                })
                .collect();
            let mut combined = Service {
                nombre: name.to_owned(),
                disponible: true,
                detalle: String::new(),
                latencia_ms: 0,
            };
            for handle in handles {
                let service = handle
                    .join()
                    .unwrap_or_else(|_| Service::down(name, "error interno"));
                if !service.disponible {
                    combined.disponible = false;
                    combined.detalle = SOME_URLS_DOWN.to_owned();
                }
                combined.latencia_ms = combined.latencia_ms.max(service.latencia_ms);
            }
            combined
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    fn listener() -> (TcpListener, String) {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", l.local_addr().expect("addr"));
        (l, url)
    }

    /// A URL whose port nothing listens on.
    fn closed_url() -> String {
        let (l, url) = listener();
        drop(l);
        url
    }

    const T: Duration = Duration::from_secs(2);

    #[test]
    fn default_ports_follow_the_scheme() {
        assert_eq!(host_and_port("http://h.test"), Some(("h.test".into(), 80)));
        assert_eq!(
            host_and_port("https://h.test"),
            Some(("h.test".into(), 443))
        );
        assert_eq!(
            host_and_port("https://h.test:9443/x"),
            Some(("h.test".into(), 9443))
        );
        assert_eq!(host_and_port("ftp://h.test"), Some(("h.test".into(), 443)));
        assert_eq!(host_and_port("http://[::1]:81"), Some(("::1".into(), 81)));
        assert_eq!(host_and_port("not a url"), None);
        assert_eq!(host_and_port("h.test:443"), None);
        assert_eq!(host_and_port("mailto:a@b.c"), None);
    }

    #[test]
    fn blank_and_malformed_urls_are_reported_without_dialing() {
        assert_eq!(check_url("S", "  ", T).detalle, "no configurado");
        assert_eq!(check_url("S", "nonsense", T).detalle, "URL invalida");
        let down = check_url("S", "", T);
        assert!(!down.disponible && down.latencia_ms == 0);
    }

    #[test]
    fn a_listening_port_is_available_and_a_closed_one_is_not() {
        let (_listener, url) = listener();
        let up = check_url("S", &url, T);
        assert!(up.disponible && up.detalle.is_empty(), "{up:?}");
        let down = check_url("S", &closed_url(), T);
        assert!(!down.disponible);
        assert!(!down.detalle.is_empty());
    }

    #[test]
    fn a_group_needs_every_url_and_keeps_the_slowest_latency() {
        let (_a, up_a) = listener();
        let (_b, up_b) = listener();
        let all = check_url_group("G", &[up_a.clone(), up_b], T);
        assert!(all.disponible && all.detalle.is_empty());
        let partial = check_url_group("G", &[up_a.clone(), closed_url()], T);
        assert!(!partial.disponible);
        assert_eq!(partial.detalle, "una o más URL no disponibles");
        let blank = check_url_group("G", &[up_a, String::new()], T);
        assert_eq!(blank.detalle, "una o más URL no disponibles");
        assert_eq!(check_url_group("G", &[], T).detalle, "no configurado");
    }

    #[test]
    fn a_probe_that_hangs_is_cut_at_the_timeout() {
        let start = Instant::now();
        let service = timed("slow", Duration::from_millis(100), || {
            thread::sleep(Duration::from_secs(2));
            Ok(())
        });
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!service.disponible);
        assert_eq!(service.detalle, "i/o timeout");
    }

    #[test]
    fn the_detail_is_omitted_from_the_json_when_empty() {
        let ok = serde_json::to_value(Service {
            nombre: "A".into(),
            disponible: true,
            detalle: String::new(),
            latencia_ms: 3,
        })
        .expect("json");
        assert_eq!(
            ok,
            serde_json::json!({"nombre":"A","disponible":true,"latencia_ms":3})
        );
    }
}
