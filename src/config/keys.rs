//! The registry of every configuration key, in documentation order.
//!
//! Phase writers: if your phase needs a new key, append it here (in its
//! section) with `secret: true` for credentials and personal data.

/// One configuration setting.
#[derive(Debug, Clone, Copy)]
pub struct KeyDef {
    /// Stable dotted name, e.g. `zonaser.password`.
    pub key: &'static str,
    /// Credentials and personal data (never printed).
    pub secret: bool,
    /// Built-in default, if any.
    pub default: Option<&'static str>,
    /// One-line description.
    pub doc: &'static str,
}

const fn k(
    key: &'static str,
    secret: bool,
    default: Option<&'static str>,
    doc: &'static str,
) -> KeyDef {
    KeyDef {
        key,
        secret,
        default,
        doc,
    }
}

/// Every key. Ported from Go `config.registry`, minus MySQL, Siesa and
/// `serve` settings, plus the keys of the Postgres/Redis/Cassandra design.
pub const KEYS: &[KeyDef] = &[
    // --- new infrastructure ---
    k(
        "database.url",
        true,
        None,
        "Postgres URL, e.g. postgres://user:pass@host:5432/tessia",
    ),
    k(
        "database.pool_size",
        false,
        Some("16"),
        "Maximum Postgres connections of this process",
    ),
    k(
        "redis.url",
        true,
        None,
        "Redis URL of the Nota Técnica collection, e.g. redis://host:6379",
    ),
    k(
        "redis.nt_prefix",
        false,
        Some("nt:"),
        "Key prefix of the Nota Técnica collection in Redis",
    ),
    k(
        "cassandra.base_url",
        false,
        None,
        "Base URL of the Cassandra storage API (object store)",
    ),
    k(
        "cassandra.api_key",
        true,
        None,
        "Bearer token of the Cassandra storage API",
    ),
    k(
        "cassandra.bucket",
        false,
        None,
        "Bucket that holds the cargue artifacts",
    ),
    k(
        "workdir.root",
        false,
        None,
        "Per-machine temp root for cargue workdirs (default <os temp>/tessia-work)",
    ),
    // --- paths ---
    k(
        "paths.base_dir",
        false,
        None,
        "Root working directory for local inputs (default ./rips-data)",
    ),
    k(
        "paths.reportes",
        false,
        None,
        "Reports dir (default <base>/Reportes); logo.png of the delivery page is read from it",
    ),
    k(
        "paths.firmas",
        false,
        None,
        "Signatures dir (default <base>/firmas)",
    ),
    // --- company ---
    k("company.nit_emisor", false, Some("806011811"), "Issuer NIT"),
    k(
        "company.codigo_prestador",
        false,
        Some("1300185002"),
        "Provider code",
    ),
    k(
        "company.nit_mutualser",
        false,
        Some("806008394"),
        "Mutual Ser NIT",
    ),
    k(
        "company.adjustment_note_prefix",
        false,
        None,
        "Adjustment/credit note prefix (legacy prefijoajuste)",
    ),
    k(
        "firma.host",
        false,
        None,
        "Signature host (legacy ipfirmaybd)",
    ),
    // --- medicar ---
    k("medicar.base_url", false, None, "Medicar base URL"),
    k(
        "medicar.products_url",
        false,
        None,
        "Medicar products report base URL (.../productosDespachados)",
    ),
    k(
        "medicar.identifier",
        true,
        None,
        "Medicar login identification (f_id)",
    ),
    k("medicar.username", false, None, "Medicar user"),
    k("medicar.password", true, None, "Medicar password"),
    k(
        "medicar.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for Medicar",
    ),
    // --- convenio ---
    k(
        "convenio.max_invoice_amount",
        false,
        None,
        "Per-invoice COP cap for convenio preparar, e.g. 100000 (legacy tessia.montoMaximoFactura)",
    ),
    // --- minsalud (SISPRO) ---
    k(
        "minsalud.base_url",
        false,
        None,
        "SISPRO service base URL, e.g. https://host:9443",
    ),
    k(
        "minsalud.tipo_doc",
        false,
        None,
        "SISPRO login document type",
    ),
    k(
        "minsalud.numero_doc",
        true,
        None,
        "SISPRO login document number",
    ),
    k("minsalud.clave", true, None, "SISPRO login password"),
    k("minsalud.nit", false, None, "SISPRO login NIT"),
    k(
        "minsalud.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for SISPRO",
    ),
    k(
        "minsalud.tls_pin_sha256",
        false,
        None,
        "Optional SHA-256 of the SISPRO server certificate to trust instead of ca_file",
    ),
    k(
        "minsalud.numero_contrato",
        false,
        None,
        "NUMERO_CONTRATO set in the FEV XML for cuv generar (replaces 22382)",
    ),
    // --- zonaser (Mutual Ser) ---
    k(
        "zonaser.login_url",
        false,
        None,
        "ZonaSer login (tocumen) URL",
    ),
    k(
        "zonaser.benavente_base_url",
        false,
        None,
        "ZonaSer benavente API base URL",
    ),
    k("zonaser.username", false, None, "ZonaSer user"),
    k("zonaser.password", true, None, "ZonaSer password"),
    k(
        "zonaser.organization_nit",
        false,
        None,
        "ZonaSer organization NIT",
    ),
    k("zonaser.user_email", true, None, "ZonaSer user email"),
    k("zonaser.user_id", true, None, "ZonaSer user id"),
    k(
        "zonaser.roles",
        false,
        None,
        "ZonaSer roles, comma-separated",
    ),
    k(
        "zonaser.organization_name",
        false,
        None,
        "ZonaSer organization name (Organizacionname header)",
    ),
    k(
        "zonaser.portal_origin",
        false,
        None,
        "ZonaSer web portal URL sent as origin/referer",
    ),
    k(
        "zonaser.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for ZonaSer",
    ),
    // --- derechos ---
    k(
        "derechos.token_url",
        false,
        None,
        "Rights validator token URL",
    ),
    k(
        "derechos.validate_url",
        false,
        None,
        "Rights validator validation URL",
    ),
    k(
        "derechos.client_id",
        false,
        None,
        "Rights validator client id",
    ),
    k(
        "derechos.client_secret",
        true,
        None,
        "Rights validator client secret",
    ),
    k("derechos.username", false, None, "Rights validator user"),
    k("derechos.password", true, None, "Rights validator password"),
    k(
        "derechos.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for the rights validator",
    ),
    // --- thefactory ---
    k(
        "thefactory.base_url",
        false,
        None,
        "The Factory HKA base URL (scheme://host only)",
    ),
    k(
        "thefactory.token_empresa",
        true,
        None,
        "The Factory company token",
    ),
    k(
        "thefactory.token_password",
        true,
        None,
        "The Factory token password",
    ),
    k(
        "thefactory.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for The Factory",
    ),
    // --- mipres ---
    k(
        "mipres.base_url",
        false,
        Some("https://wsmipres.sispro.gov.co/WSSUMMIPRESNOPBS"),
        "MIPRES NO PBS web service base URL",
    ),
    k("mipres.nit", false, None, "MIPRES provider NIT"),
    k(
        "mipres.token_ppal",
        true,
        None,
        "MIPRES primary token (the secondary token is generated from it once per run)",
    ),
    k(
        "mipres.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for MIPRES",
    ),
    // --- cpscan ---
    k("cpscan.base_url", false, None, "CPScan base URL"),
    k("cpscan.bucket_url", false, None, "CPScan bucket URL"),
    k(
        "cpscan.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for CPScan",
    ),
    k(
        "cpscan.tls_server_name",
        false,
        None,
        "Verify the CPScan certificate against this hostname while connecting to the URL host",
    ),
    // --- soporte ---
    k(
        "soporte.java_bin",
        false,
        None,
        "java executable for the Jasper sidecar (default java); only for report names without a native renderer",
    ),
    k(
        "soporte.sidecar_classpath",
        false,
        None,
        "Jasper sidecar classpath",
    ),
    k(
        "soporte.signatures_enabled",
        false,
        None,
        "true adds the signed delivery page (Formato_App); default off, requires Juridica approval",
    ),
    // --- portal mutualser ---
    k(
        "portalmutualser.base_url",
        false,
        None,
        "Mutual Ser authorization portal base URL",
    ),
    k("portalmutualser.username", false, None, "Portal user"),
    k("portalmutualser.password", true, None, "Portal password"),
    k(
        "portalmutualser.ca_file",
        false,
        None,
        "Optional PEM CA bundle to pin for the portal",
    ),
];

/// Environment names Go still accepted in a file but no code reads (plus the
/// Go settings of machinery this port does not have). A file that lists them
/// keeps loading; their values are ignored.
pub const RETIRED_ENV_NAMES: &[&str] = &[
    "TESSIA_MYSQL_RIPS_DSN",
    "TESSIA_MYSQL_ISOSMART_DSN",
    "TESSIA_MYSQL_SCAN_DSN",
    "TESSIA_MYSQL_TESSIA_DSN",
    "TESSIA_MYSQL_POOL_MAX_OPEN_CONNS",
    "TESSIA_MYSQL_POOL_MAX_IDLE_CONNS",
    "TESSIA_PATHS_EXCEL",
    "TESSIA_PATHS_FUENTES",
    "TESSIA_PATHS_AUTOMATIC",
    "TESSIA_PATHS_ARCHIVOS",
    "TESSIA_PATHS_SIESA",
    "TESSIA_ZONASER_FARAON_BASE_URL",
    "TESSIA_SERVE_ALLOWED_ORIGINS",
    "TESSIA_SIESA_BASE_URL",
    "TESSIA_SIESA_CONNI_KEY",
    "TESSIA_SIESA_CONNI_TOKEN",
    "TESSIA_SIESA_COMPANY_ID",
    "TESSIA_SIESA_CO_FACTURACION",
    "TESSIA_SIESA_BODEGA",
    "TESSIA_SIESA_CCOSTO",
    "TESSIA_SIESA_WORKERS",
    "TESSIA_SIESA_MAX_ATTEMPTS",
    "TESSIA_SIESA_TIMEOUT",
    "TESSIA_SIESA_CA_FILE",
];

/// Environment variable of a dotted key: `TESSIA_` + upper case, `.` -> `_`.
pub fn env_name(key: &str) -> String {
    format!("TESSIA_{}", key.to_uppercase().replace('.', "_"))
}

pub(super) fn by_key(key: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|def| def.key == key)
}

pub(super) fn by_env_name(name: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|def| env_name(def.key) == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_names_derive_from_keys() {
        assert_eq!(env_name("zonaser.password"), "TESSIA_ZONASER_PASSWORD");
        assert_eq!(env_name("redis.nt_prefix"), "TESSIA_REDIS_NT_PREFIX");
    }

    #[test]
    fn keys_and_env_names_are_unique() {
        let mut names: Vec<String> = KEYS.iter().map(|d| env_name(d.key)).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
        assert!(RETIRED_ENV_NAMES.iter().all(|r| by_env_name(r).is_none()));
    }
}
