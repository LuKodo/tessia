//! Locating, fetching and archiving the step files of a cargue (Go
//! `internal/adapters/invoicefs`).
//!
//! Go read a folder; here the files are the cargue's artifacts (the
//! `artefactos` index), fetched on demand into the per-machine workdir. The
//! same layout rules apply to the relative paths: the root, `XML/`, `CRC/` and,
//! for every `lote_*` directory, `lote_*/`, `lote_*/XML`, `lote_*/ZIP` and
//! `lote_*/CRC`. `Radicado/` entries and filed artifacts are ignored by the
//! finders unless asked for; `CargueAnterior/` is never indexed. "Archiving" a
//! confirmed file means recording it in `radicacion_archivo` (the bucket
//! cannot rename); the marker only holds while the artifact keeps its SHA-256.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context, Result};
use regex::Regex;

use super::archive::ArchiveStore;
use super::model::UploadFile;
use super::rules::txt_has_cuv;
use crate::domain::radicacion::Step;
use crate::storage::{Artifacts, Workdir};

/// Folder names that hold filed (archived) files / the previous upload.
const ARCHIVE_DIR: &str = "Radicado";
const PREVIOUS_UPLOAD_DIR: &str = "CargueAnterior";

/// Largest XML entry read to find the invoice total (zip bomb guard).
const MAX_XML_ENTRY: u64 = 64 << 20;

/// What [`Files::find`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    File(UploadFile),
    /// Not usable; the Spanish reason (shown to the operator).
    Missing(String),
}

/// The step files of a cargue.
pub trait Files: Send + Sync {
    /// Invoices that have at least one step file (sorted).
    fn invoices(&self) -> Vec<String>;
    /// Locates the file of `step` for `invoice`; `include_archived` also looks
    /// at filed files.
    fn find(&self, step: Step, invoice: &str, include_archived: bool) -> Result<Found>;
    /// Records a confirmed file as filed.
    fn archive(&self, file: &UploadFile) -> Result<()>;
    /// The payable total of the invoice ZIP: `(amount, source tag)`.
    fn xml_total(&self, file: &UploadFile) -> Option<(String, String)>;
}

static INVOICE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]{3}\d+$").unwrap_or_else(|_| unreachable!()));

/// Looks like an invoice number (3 letters + digits).
pub fn valid_invoice(s: &str) -> bool {
    INVOICE_RE.is_match(s)
}

/// The invoice number of a step file name (`FEV1_XML_PDF.zip`, `FEV1.json`,
/// `FEV1.txt`, `CRC_<nit>_FEV1.zip`).
pub fn invoice_from_file_name(name: &str, nit: &str) -> Option<String> {
    let mut s = name.trim().to_uppercase();
    if !nit.is_empty() {
        s = s.replacen(&format!("CRC_{}_", nit.to_uppercase()), "", 1);
    }
    for suffix in ["_XML_PDF", ".ZIP", ".JSON", ".TXT", ".XML", ".PDF"] {
        s = s.replace(suffix, "");
    }
    valid_invoice(&s).then_some(s)
}

#[derive(Debug, Clone)]
struct Entry {
    rel: String,
    name: String,
    size: u64,
    sha256: String,
    dir_rank: usize,
    /// Under `Radicado/` or marked as filed.
    archived: bool,
}

/// Rank of a directory in the index order (root, XML, CRC, lotes), or `None`
/// when the directory is not indexed.
fn dir_rank(dirs: &[&str]) -> Option<usize> {
    match dirs {
        [] => Some(0),
        ["XML"] => Some(1),
        ["CRC"] => Some(2),
        [lote] if is_lote(lote) => Some(3),
        [lote, "XML" | "ZIP" | "CRC"] if is_lote(lote) => Some(4),
        _ => None,
    }
}

fn is_lote(dir: &str) -> bool {
    dir.to_lowercase().starts_with("lote_")
}

fn has_segment(dirs: &[&str], wanted: &str) -> bool {
    dirs.iter().any(|d| d.eq_ignore_ascii_case(wanted))
}

/// The artifact files of one cargue, fetched on demand into a workdir.
pub struct ArtifactFiles {
    artifacts: Artifacts,
    archive: ArchiveStore,
    workdir: Workdir,
    cargue_id: String,
    nit: String,
    by_invoice: HashMap<String, Vec<Entry>>,
    by_rel: HashMap<String, Entry>,
    newly_archived: Mutex<HashSet<String>>,
    fetching: Mutex<HashMap<String, Arc<Mutex<bool>>>>,
}

impl ArtifactFiles {
    /// Indexes the cargue's artifacts (`artefactos` + the archive markers).
    pub fn load(
        artifacts: Artifacts,
        archive: ArchiveStore,
        workdir: Workdir,
        cargue_id: &str,
        nit: &str,
    ) -> Result<ArtifactFiles> {
        let marks = archive.list(cargue_id)?;
        let mut by_invoice: HashMap<String, Vec<Entry>> = HashMap::new();
        let mut by_rel = HashMap::new();
        for row in artifacts.list(cargue_id)? {
            let Some(entry) = classify(&row.rel_path, row.size, &row.sha256, &marks, nit) else {
                continue;
            };
            let name = entry.name.clone();
            by_rel.insert(entry.rel.clone(), entry.clone());
            if let Some(invoice) = invoice_from_file_name(&name, nit) {
                by_invoice.entry(invoice).or_default().push(entry);
            }
        }
        for entries in by_invoice.values_mut() {
            entries.sort_by(|a, b| (a.dir_rank, &a.rel).cmp(&(b.dir_rank, &b.rel)));
        }
        Ok(ArtifactFiles {
            artifacts,
            archive,
            workdir,
            cargue_id: cargue_id.to_owned(),
            nit: nit.to_owned(),
            by_invoice,
            by_rel,
            newly_archived: Mutex::new(HashSet::new()),
            fetching: Mutex::new(HashMap::new()),
        })
    }

    /// The workdir the files are fetched into.
    pub fn workdir(&self) -> &Workdir {
        &self.workdir
    }

    fn is_archived(&self, entry: &Entry) -> bool {
        entry.archived
            || self
                .newly_archived
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains(&entry.rel)
    }

    /// Downloads the artifact once (concurrent callers share the download).
    fn ensure_local(&self, entry: &Entry) -> Result<PathBuf> {
        let path = self.workdir.file(&entry.rel)?;
        let slot = {
            let mut map = self.fetching.lock().unwrap_or_else(|p| p.into_inner());
            map.entry(entry.rel.clone()).or_default().clone()
        };
        let mut fetched = slot.lock().unwrap_or_else(|p| p.into_inner());
        if !*fetched {
            self.artifacts
                .fetch_to_file(&self.cargue_id, &entry.rel, &path)?;
            *fetched = true;
        }
        Ok(path)
    }

    /// Checks the name guard and size of a candidate and fetches it.
    fn accept(&self, entry: &Entry, invoice: &str) -> Result<Found> {
        if invoice_from_file_name(&entry.name, &self.nit).as_deref() != Some(invoice) {
            return Ok(Found::Missing(format!(
                "el archivo {} no corresponde a la factura {invoice}",
                entry.name
            )));
        }
        if entry.size == 0 {
            return Ok(Found::Missing(format!(
                "el archivo {} esta vacio",
                entry.name
            )));
        }
        let path = self.ensure_local(entry)?;
        Ok(Found::File(UploadFile {
            invoice: invoice.to_owned(),
            rel: entry.rel.clone(),
            path,
            name: entry.name.clone(),
            size: entry.size,
        }))
    }

    /// The `contains` and `ends with` parts of a step file name (upper case).
    fn pattern(&self, step: Step, invoice: &str) -> Option<(String, &'static str)> {
        match step {
            Step::InvoiceZip => Some((invoice.to_uppercase(), "_XML_PDF.ZIP")),
            Step::Rips => Some((invoice.to_uppercase(), ".JSON")),
            Step::Crc => Some((format!("CRC_{}_{invoice}", self.nit).to_uppercase(), ".ZIP")),
            Step::Cuv => None,
        }
    }

    fn candidates(&self, invoice: &str) -> &[Entry] {
        self.by_invoice.get(invoice).map_or(&[], Vec::as_slice)
    }

    /// The TXT carrying a valid CUV (filed ones included, like the legacy search).
    fn find_cuv(&self, invoice: &str) -> Result<Found> {
        let mut candidates: Vec<&Entry> = self
            .by_rel
            .values()
            .filter(|e| {
                let n = e.name.to_uppercase();
                n.contains(invoice) && n.ends_with(".TXT")
            })
            .collect();
        candidates
            .sort_by(|a, b| (txt_priority(&a.name), &a.rel).cmp(&(txt_priority(&b.name), &b.rel)));
        if candidates.is_empty() {
            return Ok(Found::Missing("TXT=FALTA".into()));
        }
        let mut diag = Vec::new();
        for entry in candidates {
            if error_txt(&entry.name.to_uppercase()) {
                diag.push(format!("{}=ERROR", entry.name));
                continue;
            }
            let content = if entry.size == 0 {
                Vec::new()
            } else {
                self.artifacts.fetch_bytes(&self.cargue_id, &entry.rel)?
            };
            if content.is_empty() || !txt_has_cuv(&content) {
                diag.push(format!("{}=SIN_CUV_VALIDO", entry.name));
                continue;
            }
            match self.accept(entry, invoice)? {
                found @ Found::File(_) => return Ok(found),
                Found::Missing(_) => diag.push(format!("{}=NO_CORRESPONDE", entry.name)),
            }
        }
        Ok(Found::Missing(format!(
            "TXT candidatos: {}",
            diag.join(", ")
        )))
    }
}

/// Indexes a relative path, or `None` when it is never indexed.
fn classify(
    rel: &str,
    size: i64,
    sha256: &str,
    marks: &HashMap<String, String>,
    nit: &str,
) -> Option<Entry> {
    let mut parts: Vec<&str> = rel.split('/').collect();
    let name = parts.pop()?.to_owned();
    if has_segment(&parts, PREVIOUS_UPLOAD_DIR) {
        return None;
    }
    let in_archive = has_segment(&parts, ARCHIVE_DIR);
    let rank = if in_archive { 0 } else { dir_rank(&parts)? };
    let _ = nit;
    Some(Entry {
        rel: rel.to_owned(),
        name,
        size: size.max(0) as u64,
        sha256: sha256.to_owned(),
        dir_rank: rank,
        archived: in_archive || marks.get(rel).is_some_and(|m| m == sha256),
    })
}

fn error_txt(upper_name: &str) -> bool {
    ["_ERROR", "_RESPUESTA_NO_CORRESPONDE", "_JSON_ERROR"]
        .iter()
        .any(|marker| upper_name.contains(marker))
}

static EXACT_TXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]{2,4}\d+\.TXT$").unwrap_or_else(|_| unreachable!()));

fn txt_priority(name: &str) -> u8 {
    let n = name.to_uppercase();
    if EXACT_TXT.is_match(&n) {
        0
    } else if error_txt(&n) {
        2
    } else {
        1
    }
}

impl Files for ArtifactFiles {
    fn invoices(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .by_invoice
            .iter()
            .filter(|(_, entries)| entries.iter().any(|e| !self.is_archived(e)))
            .map(|(invoice, _)| invoice.clone())
            .collect();
        out.sort();
        out
    }

    fn find(&self, step: Step, invoice: &str, include_archived: bool) -> Result<Found> {
        let invoice = crate::domain::normalize_invoice(invoice);
        let Some((contains, suffix)) = self.pattern(step, &invoice) else {
            return self.find_cuv(&invoice);
        };
        let matching = |archived: bool| -> Option<&Entry> {
            self.candidates(&invoice).iter().find(|e| {
                let n = e.name.to_uppercase();
                self.is_archived(e) == archived && n.contains(&contains) && n.ends_with(suffix)
            })
        };
        let found = matching(false).or_else(|| include_archived.then(|| matching(true)).flatten());
        match found {
            Some(entry) => self.accept(entry, &invoice),
            None => Ok(Found::Missing(format!(
                "no se encontro archivo {} para factura {invoice}",
                step.label()
            ))),
        }
    }

    fn archive(&self, file: &UploadFile) -> Result<()> {
        let Some(entry) = self.by_rel.get(&file.rel) else {
            return Ok(());
        };
        if self.is_archived(entry) {
            return Ok(());
        }
        self.archive
            .mark(&self.cargue_id, &entry.rel, &entry.sha256)
            .with_context(|| format!("no se pudo archivar {}", entry.rel))?;
        self.newly_archived
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(entry.rel.clone());
        Ok(())
    }

    fn xml_total(&self, file: &UploadFile) -> Option<(String, String)> {
        zip_xml_total(&file.path)
    }
}

static CDATA_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<cbc:Description>\s*<!\[CDATA\[(.*?)\]\]>\s*</cbc:Description>")
        .unwrap_or_else(|_| unreachable!())
});

fn amount_re(tag: &str) -> Regex {
    Regex::new(&format!(
        r"(?is)<cac:LegalMonetaryTotal\b[^>]*>.*?<cbc:{tag}\b[^>]*>\s*([0-9.,]+)\s*</cbc:{tag}>.*?</cac:LegalMonetaryTotal>"
    ))
    .unwrap_or_else(|_| unreachable!())
}

static AMOUNT_RES: LazyLock<[(&'static str, Regex); 2]> = LazyLock::new(|| {
    [
        ("PayableAmount", amount_re("PayableAmount")),
        ("TaxInclusiveAmount", amount_re("TaxInclusiveAmount")),
    ]
});

/// Reads the invoice embedded as CDATA in the AttachedDocument XML of the ZIP
/// and returns its `PayableAmount` (or `TaxInclusiveAmount`) with thousands
/// separators removed, plus the tag it came from.
pub fn zip_xml_total(path: &std::path::Path) -> Option<(String, String)> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else {
            continue;
        };
        let is_xml = entry
            .name()
            .is_ok_and(|name| name.to_lowercase().ends_with(".xml"));
        if entry.is_dir() || !is_xml {
            continue;
        }
        let mut content = Vec::new();
        if entry
            .take(MAX_XML_ENTRY + 1)
            .read_to_end(&mut content)
            .is_err()
            || content.len() as u64 > MAX_XML_ENTRY
        {
            continue;
        }
        let text = String::from_utf8_lossy(&content);
        let Some(cdata) = CDATA_RE.captures(&text) else {
            continue;
        };
        for (tag, re) in AMOUNT_RES.iter() {
            if let Some(found) = re.captures(&cdata[1]) {
                return Some((found[1].replace(',', ""), (*tag).to_owned()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::db::test_support::TestDb;
    use crate::storage::MemoryStore;

    const NIT: &str = "900111222";
    const CARGUE: &str = "Convenio_21102026_21102026";

    #[test]
    fn invoice_names_are_extracted_from_step_files() {
        for (name, want) in [
            ("FEV123_XML_PDF.zip", Some("FEV123")),
            ("fev123.json", Some("FEV123")),
            ("FEV123.txt", Some("FEV123")),
            ("CRC_900111222_FEV123.zip", Some("FEV123")),
            ("FEV123.xml", Some("FEV123")),
            ("FEV123.pdf", Some("FEV123")),
            ("FEV123_ERROR.txt", None),
            ("README.md", None),
            ("AB123.json", None),
            ("CRC_111_FEV123.zip", None),
        ] {
            assert_eq!(invoice_from_file_name(name, NIT).as_deref(), want, "{name}");
        }
    }

    fn zip_with_invoice(inner: &str) -> Vec<u8> {
        let xml = format!(
            "<AttachedDocument><cbc:Description><![CDATA[{inner}]]></cbc:Description></AttachedDocument>"
        );
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zw = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();
        zw.start_file("FEV1.pdf", options).expect("pdf");
        zw.write_all(b"%PDF-").expect("pdf bytes");
        zw.start_file("FEV1.xml", options).expect("xml");
        zw.write_all(xml.as_bytes()).expect("xml bytes");
        zw.finish().expect("finish");
        buf.into_inner()
    }

    fn totals(tag_block: &str) -> Vec<u8> {
        zip_with_invoice(&format!(
            "<Invoice><cac:LegalMonetaryTotal><cbc:LineExtensionAmount currencyID=\"COP\">1.00</cbc:LineExtensionAmount>{tag_block}</cac:LegalMonetaryTotal></Invoice>"
        ))
    }

    fn total_of(bytes: &[u8]) -> Option<(String, String)> {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("a.zip");
        std::fs::write(&path, bytes).expect("write");
        zip_xml_total(&path)
    }

    #[test]
    fn xml_total_reads_payable_then_tax_inclusive_without_commas() {
        let payable =
            totals("<cbc:PayableAmount currencyID=\"COP\"> 1,234,567.50 </cbc:PayableAmount>");
        assert_eq!(
            total_of(&payable),
            Some(("1234567.50".into(), "PayableAmount".into()))
        );
        let tax =
            totals("<cbc:TaxInclusiveAmount currencyID=\"COP\">900.00</cbc:TaxInclusiveAmount>");
        assert_eq!(
            total_of(&tax),
            Some(("900.00".into(), "TaxInclusiveAmount".into()))
        );
        let both = totals(
            "<cbc:TaxInclusiveAmount>5.00</cbc:TaxInclusiveAmount><cbc:PayableAmount>7.00</cbc:PayableAmount>",
        );
        assert_eq!(
            total_of(&both),
            Some(("7.00".into(), "PayableAmount".into()))
        );
        assert_eq!(total_of(&zip_with_invoice("<Invoice/>")), None);
        assert_eq!(total_of(b"not a zip"), None);
    }

    struct Fixture {
        _db: TestDb,
        files: ArtifactFiles,
        artifacts: Artifacts,
        archive: ArchiveStore,
        ctx_root: tempfile::TempDir,
    }

    fn fixture(entries: &[(&str, &[u8])]) -> Fixture {
        let db = TestDb::new();
        let artifacts = Artifacts::new(
            Arc::new(MemoryStore::new()),
            crate::stores::artefactos::ArtefactosStore::new(db.pool()),
        );
        for (rel, data) in entries {
            artifacts.publish_bytes(CARGUE, rel, data).expect("publish");
        }
        let archive = ArchiveStore::new(db.pool());
        let ctx_root = tempfile::tempdir().expect("root");
        let workdir = Workdir::create(ctx_root.path(), CARGUE).expect("workdir");
        let files = ArtifactFiles::load(artifacts.clone(), archive.clone(), workdir, CARGUE, NIT)
            .expect("load");
        Fixture {
            _db: db,
            files,
            artifacts,
            archive,
            ctx_root,
        }
    }

    fn file(found: Found) -> UploadFile {
        match found {
            Found::File(f) => f,
            Found::Missing(why) => panic!("expected a file, got: {why}"),
        }
    }

    fn missing(found: Found) -> String {
        match found {
            Found::Missing(why) => why,
            Found::File(f) => panic!("expected a miss, got {}", f.rel),
        }
    }

    #[test]
    fn finds_and_fetches_the_four_step_files() {
        let cuv: &[u8] = br#"{"CUV":"abc"}"#;
        let fx = fixture(&[
            ("XML/FEV1_XML_PDF.zip", b"zip"),
            ("FEV1.txt", cuv),
            ("FEV1.json", b"{}"),
            ("CRC/CRC_900111222_FEV1.zip", b"crc"),
            ("derechos/FEV1.json", b"{\"not\":\"rips\"}"),
        ]);
        assert_eq!(fx.files.invoices(), ["FEV1"]);
        let zip = file(
            fx.files
                .find(Step::InvoiceZip, "fev-1", false)
                .expect("find"),
        );
        assert_eq!(
            (zip.rel.as_str(), zip.name.as_str(), zip.size),
            ("XML/FEV1_XML_PDF.zip", "FEV1_XML_PDF.zip", 3)
        );
        assert_eq!(std::fs::read(&zip.path).expect("read"), b"zip");
        assert!(zip.path.starts_with(fx.files.workdir().path()));
        assert_eq!(
            file(fx.files.find(Step::Cuv, "FEV1", false).expect("find")).rel,
            "FEV1.txt"
        );
        let rips = file(fx.files.find(Step::Rips, "FEV1", false).expect("find"));
        assert_eq!(
            rips.rel, "FEV1.json",
            "a JSON in a non-indexed folder is never the RIPS"
        );
        assert_eq!(
            file(fx.files.find(Step::Crc, "FEV1", false).expect("find")).rel,
            "CRC/CRC_900111222_FEV1.zip"
        );
        assert!(fx.ctx_root.path().exists());
    }

    #[test]
    fn misses_explain_themselves_in_spanish() {
        let fx = fixture(&[
            ("XML/FEV1_XML_PDF.zip", b""),
            ("CRC/CRC_111_FEV1.zip", b"x"),
        ]);
        assert_eq!(
            missing(
                fx.files
                    .find(Step::InvoiceZip, "FEV1", false)
                    .expect("find")
            ),
            "el archivo FEV1_XML_PDF.zip esta vacio"
        );
        assert_eq!(
            missing(fx.files.find(Step::Rips, "FEV1", false).expect("find")),
            "no se encontro archivo JSON para factura FEV1"
        );
        assert_eq!(
            missing(fx.files.find(Step::Crc, "FEV1", false).expect("find")),
            "no se encontro archivo ZIP CRC para factura FEV1"
        );
        assert_eq!(
            missing(fx.files.find(Step::Cuv, "FEV1", false).expect("find")),
            "TXT=FALTA"
        );
    }

    #[test]
    fn txt_candidates_need_a_valid_cuv_and_error_txts_are_skipped() {
        let fx = fixture(&[
            ("FEV1.txt", b"{\"CUV\":\"-\"}"),
            ("FEV1_ERROR.txt", b"{\"CUV\":\"abc\"}"),
            ("lote_1/FEV1.txt", b"{\"CodigoUnicoValidacion\":\"zzz\"}"),
        ]);
        assert_eq!(
            file(fx.files.find(Step::Cuv, "FEV1", false).expect("find")).rel,
            "lote_1/FEV1.txt"
        );
        let bad = fixture(&[("FEV1.txt", b"{\"CUV\":\"-\"}"), ("FEV1_ERROR.txt", b"{}")]);
        assert_eq!(
            missing(bad.files.find(Step::Cuv, "FEV1", false).expect("find")),
            "TXT candidatos: FEV1.txt=SIN_CUV_VALIDO, FEV1_ERROR.txt=ERROR"
        );
    }

    #[test]
    fn lote_folders_are_indexed_and_other_folders_are_not() {
        let fx = fixture(&[
            ("lote_2/ZIP/FEV2_XML_PDF.zip", b"z"),
            ("lote_2/XML/FEV3_XML_PDF.zip", b"z"),
            ("otra/FEV4_XML_PDF.zip", b"z"),
            ("CargueAnterior/FEV5_XML_PDF.zip", b"z"),
            ("Radicado/XML/FEV6_XML_PDF.zip", b"z"),
        ]);
        assert_eq!(fx.files.invoices(), ["FEV2", "FEV3"]);
        assert_eq!(
            file(
                fx.files
                    .find(Step::InvoiceZip, "FEV2", false)
                    .expect("find")
            )
            .rel,
            "lote_2/ZIP/FEV2_XML_PDF.zip"
        );
        assert!(matches!(
            fx.files
                .find(Step::InvoiceZip, "FEV4", false)
                .expect("find"),
            Found::Missing(_)
        ));
        assert!(matches!(
            fx.files.find(Step::InvoiceZip, "FEV5", true).expect("find"),
            Found::Missing(_)
        ));
        assert!(matches!(
            fx.files
                .find(Step::InvoiceZip, "FEV6", false)
                .expect("find"),
            Found::Missing(_)
        ));
        assert_eq!(
            file(fx.files.find(Step::InvoiceZip, "FEV6", true).expect("find")).rel,
            "Radicado/XML/FEV6_XML_PDF.zip"
        );
    }

    #[test]
    fn archiving_hides_a_file_until_it_is_asked_for_or_regenerated() {
        let fx = fixture(&[("XML/FEV1_XML_PDF.zip", b"v1")]);
        let zip = file(
            fx.files
                .find(Step::InvoiceZip, "FEV1", false)
                .expect("find"),
        );
        fx.files.archive(&zip).expect("archive");
        fx.files.archive(&zip).expect("archive twice is fine");
        assert!(fx.files.invoices().is_empty());
        assert!(matches!(
            fx.files
                .find(Step::InvoiceZip, "FEV1", false)
                .expect("find"),
            Found::Missing(_)
        ));
        assert_eq!(
            file(fx.files.find(Step::InvoiceZip, "FEV1", true).expect("find")).rel,
            zip.rel
        );
        let marks = fx.archive.list(CARGUE).expect("marks");
        assert_eq!(marks.len(), 1);

        // A new run sees the marker; a regenerated artifact is a new, unfiled file.
        let reload = |fx: &Fixture| {
            let workdir = Workdir::create(fx.ctx_root.path(), CARGUE).expect("workdir");
            ArtifactFiles::load(
                fx.artifacts.clone(),
                fx.archive.clone(),
                workdir,
                CARGUE,
                NIT,
            )
            .expect("load")
        };
        assert!(reload(&fx).invoices().is_empty());
        fx.artifacts
            .publish_bytes(CARGUE, "XML/FEV1_XML_PDF.zip", b"v2")
            .expect("republish");
        assert_eq!(reload(&fx).invoices(), ["FEV1"]);
    }

    #[test]
    fn archived_txt_still_serves_the_cuv_search() {
        let fx = fixture(&[("FEV1.txt", b"{\"CUV\":\"abc\"}")]);
        let txt = file(fx.files.find(Step::Cuv, "FEV1", false).expect("find"));
        fx.files.archive(&txt).expect("archive");
        assert_eq!(
            file(fx.files.find(Step::Cuv, "FEV1", false).expect("find")).rel,
            "FEV1.txt"
        );
    }
}
