//! Invoice selection and validation: the distinct affiliates of the selected
//! invoices are resolved (rights cache first, then the validator, in
//! parallel, each once) and every invoice is classified.
//!
//! Port of Go `internal/derechos/service.go`, over this module's own input
//! structs ([`InvoiceInput`]) so it does not depend on how the dataset is
//! stored.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;

use super::availability::Stats;
use super::client::{ValidatorClient, ValidatorError};
use super::error::DerechosError;
use super::rights::{
    AffiliateQuery, REASON_INCOMPLETE, REASON_NO_DOCUMENT, REASON_NOT_FOUND, REASON_UNAVAILABLE,
    Rights, cross_check,
};
use crate::domain::{Outcome, PhaseState, normalize_invoice};
use crate::shutdown;
use crate::stores::derechos::{DerechosCache, Rights as CachedRights};
use crate::stores::proceso::ProcesoState;
use crate::util::par;

/// Rights obtained from the validator.
pub const SOURCE_VALIDATOR: &str = "validador";
/// Rights reused from the `derechos_afiliado` cache.
pub const SOURCE_CACHE: &str = "cache";
/// Rights taken from the file itself (prepared-format loads).
pub const SOURCE_FILE: &str = "archivo";

/// One line of an invoice with the cells this phase reads (raw, untrimmed).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    /// Affiliate document type and number.
    pub doc_type: String,
    pub doc_number: String,
    /// CSV sex and birth date (only cross-checked against the validator).
    pub csv_gender: String,
    pub csv_birth_date: String,
    /// Prepared-format loads only: `Causal exento` = régimen code.
    pub causal_exento: String,
    /// Prepared-format loads only: `Departamento` = DANE code.
    pub departamento: String,
    /// Prepared-format loads only: `Municipio` = zona.
    pub municipio: String,
}

/// An invoice with its lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvoiceInput {
    /// Normalized invoice number.
    pub invoice: String,
    pub lines: Vec<LineInput>,
}

impl InvoiceInput {
    /// The single affiliate of the invoice (`convenio preparar` holds
    /// invoices with several). An invoice without lines has an empty
    /// affiliate, i.e. no document.
    pub fn affiliate(&self) -> Result<AffiliateQuery, DerechosError> {
        let mut affiliate = AffiliateQuery::normalized("", "");
        for (index, line) in self.lines.iter().enumerate() {
            let current = AffiliateQuery::normalized(&line.doc_type, &line.doc_number);
            if index > 0 && current != affiliate {
                return Err(DerechosError::MultipleAffiliates {
                    invoice: self.invoice.clone(),
                });
            }
            affiliate = current;
        }
        Ok(affiliate)
    }
}

/// The affiliate of a `DONE` invoice with its rights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffiliateRights {
    pub affiliate: AffiliateQuery,
    pub rights: Rights,
    /// [`SOURCE_VALIDATOR`], [`SOURCE_CACHE`] or [`SOURCE_FILE`].
    pub source: &'static str,
    /// Set when the validator answered for another document type.
    pub queried_doc_type: Option<String>,
}

/// The per-invoice result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceResult {
    pub invoice: String,
    /// `DONE` or `REVISION_MANUAL`.
    pub state: PhaseState,
    pub reason: String,
    /// Set when the state is `DONE`.
    pub affiliate: Option<AffiliateRights>,
    /// Fields where the CSV disagrees with the validator.
    pub mismatches: Vec<String>,
}

impl InvoiceResult {
    pub(super) fn manual_review(invoice: &str, reason: String) -> Self {
        Self {
            invoice: invoice.to_owned(),
            state: PhaseState::RevisionManual,
            reason,
            affiliate: None,
            mismatches: Vec::new(),
        }
    }

    pub(super) fn done(invoice: &str, affiliate: AffiliateRights) -> Self {
        Self {
            invoice: invoice.to_owned(),
            state: PhaseState::Done,
            reason: String::new(),
            affiliate: Some(affiliate),
            mismatches: Vec::new(),
        }
    }

    /// What is stored in `proceso_estado`.
    pub fn outcome(&self) -> Outcome {
        Outcome::new(self.state, self.reason.clone())
    }
}

/// Tuning of [`validate`].
#[derive(Debug, Clone, Copy)]
pub struct ValidateOptions {
    /// Parallel validator calls (default 30).
    pub concurrency: usize,
    /// Cache freshness; zero does not read the cache.
    pub cache_ttl: Duration,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        Self {
            concurrency: 30,
            cache_ttl: Duration::from_secs(24 * 3600),
        }
    }
}

/// What a validation run produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validation {
    /// One result per resolved invoice, in input order.
    pub results: Vec<InvoiceResult>,
    pub stats: Stats,
    /// Set when the validator rejected the credentials: `results` holds only
    /// the invoices resolved before the stop (plus the ones without a
    /// document); the others are untouched.
    pub auth_failure: Option<String>,
}

/// Picks the invoices to validate. With `only`, exactly those (known)
/// invoices; otherwise every invoice whose derechos state is not `DONE` or
/// `REVISION_MANUAL` (manual review is only retried on explicit request).
/// The second value lists requested invoices absent from `invoices`, sorted.
pub fn select(
    invoices: Vec<InvoiceInput>,
    previous: &HashMap<String, ProcesoState>,
    only: &[String],
) -> (Vec<InvoiceInput>, Vec<String>) {
    let wanted: HashSet<String> = only
        .iter()
        .map(|s| normalize_invoice(s))
        .filter(|s| !s.is_empty())
        .collect();
    let mut found = HashSet::new();
    let mut selected = Vec::new();
    for input in invoices {
        if !wanted.is_empty() {
            if wanted.contains(&input.invoice) {
                found.insert(input.invoice.clone());
                selected.push(input);
            }
            continue;
        }
        let skip = previous
            .get(&input.invoice)
            .is_some_and(|s| matches!(s.state, PhaseState::Done | PhaseState::RevisionManual));
        if !skip {
            selected.push(input);
        }
    }
    let mut unknown: Vec<String> = wanted.difference(&found).cloned().collect();
    unknown.sort();
    (selected, unknown)
}

/// The distinct affiliates with a document, in first-seen order.
pub fn distinct_affiliates(owners: &[AffiliateQuery]) -> Vec<AffiliateQuery> {
    let mut seen = HashSet::new();
    owners
        .iter()
        .filter(|a| a.has_document() && seen.insert(a.key()))
        .cloned()
        .collect()
}

/// An affiliate's resolution, as classified for its invoices.
#[derive(Debug, Clone)]
struct Resolved {
    rights: Rights,
    source: &'static str,
    queried_doc_type: Option<String>,
    /// Manual-review reason; `None` when the rights are complete.
    reason: Option<String>,
    /// Technical validator failure (as opposed to "not found"/incomplete).
    unavailable: bool,
}

enum Resolution {
    Resolved(Resolved),
    /// The validator rejected the credentials: the run stops.
    AuthFailed(String),
    /// Skipped or cancelled by the stop: no answer about this affiliate.
    Aborted,
}

/// Resolves the affiliate of every invoice (cache first, then the validator,
/// in parallel; each distinct affiliate once) and classifies each invoice:
/// `DONE` when its affiliate is found with complete data, otherwise
/// `REVISION_MANUAL`.
///
/// An invoice belongs to exactly one affiliate; a dataset that breaks this
/// fails before any query. When the validator rejects the token even after
/// the client renewed it, the run stops: no further affiliate is dispatched,
/// the results of the invoices already resolved are returned with
/// `auth_failure` set, and the unresolved ones get no result (they are not
/// counted as unavailable).
pub fn validate(
    invoices: &[InvoiceInput],
    validator: &ValidatorClient,
    cache: Option<&DerechosCache>,
    options: &ValidateOptions,
) -> Result<Validation, DerechosError> {
    let owners = invoices
        .iter()
        .map(InvoiceInput::affiliate)
        .collect::<Result<Vec<_>, _>>()?;
    let affiliates = distinct_affiliates(&owners);
    let stop = AtomicBool::new(false);
    let auth_failure: Mutex<Option<String>> = Mutex::new(None);
    let resolutions = par::map(
        affiliates.clone(),
        options.concurrency.max(1),
        |affiliate| {
            let resolution = resolve(&affiliate, validator, cache, options, &stop);
            if let Resolution::AuthFailed(detail) = &resolution {
                let mut slot = auth_failure
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                slot.get_or_insert_with(|| detail.clone());
                stop.store(true, Ordering::SeqCst);
            }
            resolution
        },
    );
    if shutdown::requested() {
        return Err(DerechosError::Interrupted);
    }
    let auth_failure = auth_failure
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let resolved: HashMap<String, Resolved> = affiliates
        .iter()
        .zip(resolutions)
        .filter_map(|(affiliate, resolution)| match resolution {
            Resolution::Resolved(r) => Some((affiliate.key(), r)),
            _ => None,
        })
        .collect();
    Ok(Validation {
        stats: stats_of(affiliates.len(), &resolved),
        results: classify(invoices, &owners, &resolved),
        auth_failure,
    })
}

fn stats_of(affiliates: usize, resolved: &HashMap<String, Resolved>) -> Stats {
    let cached = resolved
        .values()
        .filter(|r| r.source == SOURCE_CACHE)
        .count();
    let unavailable = resolved
        .values()
        .filter(|r| r.source != SOURCE_CACHE && r.unavailable)
        .count();
    Stats {
        affiliates,
        cached,
        queried: resolved.len() - cached,
        unavailable,
    }
}

fn classify(
    invoices: &[InvoiceInput],
    owners: &[AffiliateQuery],
    resolved: &HashMap<String, Resolved>,
) -> Vec<InvoiceResult> {
    let mut results = Vec::with_capacity(invoices.len());
    for (input, owner) in invoices.iter().zip(owners) {
        if !owner.has_document() {
            results.push(InvoiceResult::manual_review(
                &input.invoice,
                REASON_NO_DOCUMENT.to_owned(),
            ));
            continue;
        }
        let Some(r) = resolved.get(&owner.key()) else {
            continue; // the run stopped before this affiliate was answered
        };
        if let Some(reason) = &r.reason {
            results.push(InvoiceResult::manual_review(&input.invoice, reason.clone()));
            continue;
        }
        let mut result = InvoiceResult::done(
            &input.invoice,
            AffiliateRights {
                affiliate: owner.clone(),
                rights: r.rights.clone(),
                source: r.source,
                queried_doc_type: r.queried_doc_type.clone(),
            },
        );
        result.mismatches = csv_mismatches(input, &r.rights);
        if !result.mismatches.is_empty() {
            // Field names only: the values are personal data.
            tracing::warn!(
                invoice = %result.invoice,
                fields = %result.mismatches.join(","),
                "convenio CSV differs from the rights validator (validator data is used)"
            );
        }
        results.push(result);
    }
    results
}

fn csv_mismatches(input: &InvoiceInput, rights: &Rights) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    for line in &input.lines {
        for field in cross_check(rights, &line.csv_gender, &line.csv_birth_date) {
            if !fields.iter().any(|f| f == field) {
                fields.push(field.to_owned());
            }
        }
    }
    fields
}

fn resolve(
    affiliate: &AffiliateQuery,
    validator: &ValidatorClient,
    cache: Option<&DerechosCache>,
    options: &ValidateOptions,
    stop: &AtomicBool,
) -> Resolution {
    if stop.load(Ordering::SeqCst) || shutdown::requested() {
        return Resolution::Aborted;
    }
    if let Some(hit) = cache_hit(affiliate, cache, options.cache_ttl) {
        return Resolution::Resolved(hit);
    }
    match validator.validate(affiliate) {
        Err(_) if stop.load(Ordering::SeqCst) || shutdown::requested() => Resolution::Aborted,
        Err(ValidatorError::Authentication(detail)) => Resolution::AuthFailed(detail),
        Err(ValidatorError::Technical(detail)) => {
            tracing::warn!(doc_type = %affiliate.doc_type, error = %detail, "rights validator failed");
            Resolution::Resolved(failed(REASON_UNAVAILABLE.to_owned(), true))
        }
        Ok(lookup) => Resolution::Resolved(answered(affiliate, lookup, cache)),
    }
}

fn failed(reason: String, unavailable: bool) -> Resolved {
    Resolved {
        rights: Rights::default(),
        source: SOURCE_VALIDATOR,
        queried_doc_type: None,
        reason: Some(reason),
        unavailable,
    }
}

fn cache_hit(
    affiliate: &AffiliateQuery,
    cache: Option<&DerechosCache>,
    ttl: Duration,
) -> Option<Resolved> {
    let cache = cache.filter(|_| !ttl.is_zero())?;
    let not_before = chrono::Duration::from_std(ttl)
        .ok()
        .and_then(|ttl| Utc::now().checked_sub_signed(ttl))?;
    match cache.get(&affiliate.doc_type, &affiliate.doc_number, not_before) {
        Ok(Some(cached)) => {
            let rights = Rights::from(cached);
            rights.missing().is_empty().then_some(Resolved {
                rights,
                source: SOURCE_CACHE,
                queried_doc_type: None,
                reason: None,
                unavailable: false,
            })
        }
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = %err, "rights cache read failed; querying the validator");
            None
        }
    }
}

fn answered(
    affiliate: &AffiliateQuery,
    lookup: super::client::Lookup,
    cache: Option<&DerechosCache>,
) -> Resolved {
    let Some(rights) = lookup.rights else {
        return failed(REASON_NOT_FOUND.to_owned(), false);
    };
    let missing = rights.missing();
    if !missing.is_empty() {
        let reason = format!("{REASON_INCOMPLETE}{}", missing.join(", "));
        return failed(reason, false);
    }
    if let Some(cache) = cache
        && let Err(err) = cache.put(
            &affiliate.doc_type,
            &affiliate.doc_number,
            &CachedRights::from(&rights),
            Utc::now(),
        )
    {
        tracing::warn!(error = %err, "rights cache write failed");
    }
    Resolved {
        rights,
        source: SOURCE_VALIDATOR,
        queried_doc_type: (lookup.queried_doc_type != affiliate.doc_type)
            .then_some(lookup.queried_doc_type),
        reason: None,
        unavailable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(doc_type: &str, doc_number: &str) -> LineInput {
        LineInput {
            doc_type: doc_type.into(),
            doc_number: doc_number.into(),
            ..LineInput::default()
        }
    }

    fn invoice(name: &str, lines: Vec<LineInput>) -> InvoiceInput {
        InvoiceInput {
            invoice: name.into(),
            lines,
        }
    }

    fn state_of(invoice: &str, state: PhaseState) -> (String, ProcesoState) {
        (
            invoice.to_owned(),
            ProcesoState {
                invoice: invoice.into(),
                phase: crate::domain::Phase::Derechos,
                state,
                motivo: String::new(),
                intentos: 1,
                version: 1,
                cargue_id: None,
                claimed_by: None,
                claimed_at: None,
                next_attempt_at: None,
                updated_at: Utc::now(),
            },
        )
    }

    #[test]
    fn an_invoice_has_exactly_one_affiliate() {
        let ok = invoice("A1", vec![line(" cc ", " 1 "), line("CC", "1")]);
        assert_eq!(ok.affiliate().expect("one").key(), "CC|1");
        let two = invoice("A2", vec![line("CC", "1"), line("CC", "2")]);
        let err = two.affiliate().expect_err("two affiliates");
        let text = err.to_string();
        assert!(text.contains("A2") && text.contains("MULTIPLES_AFILIADOS_EN_FACTURA"));
        assert!(!text.contains("CC"), "no documents in the message");
        assert!(
            !invoice("A3", vec![])
                .affiliate()
                .expect("none")
                .has_document()
        );
    }

    #[test]
    fn select_skips_done_and_manual_review_unless_named() {
        let all = vec![
            invoice("A1", vec![]),
            invoice("A2", vec![]),
            invoice("A3", vec![]),
            invoice("A4", vec![]),
        ];
        let previous: HashMap<_, _> = [
            state_of("A1", PhaseState::Done),
            state_of("A2", PhaseState::RevisionManual),
            state_of("A3", PhaseState::EnProgreso),
        ]
        .into_iter()
        .collect();
        let (selected, unknown) = select(all.clone(), &previous, &[]);
        let names: Vec<_> = selected.iter().map(|i| i.invoice.as_str()).collect();
        assert_eq!(names, ["A3", "A4"]);
        assert!(unknown.is_empty());

        let (selected, unknown) = select(
            all,
            &previous,
            &["a-1".into(), "A2".into(), "zz9".into(), "ZZ9".into()],
        );
        let names: Vec<_> = selected.iter().map(|i| i.invoice.as_str()).collect();
        assert_eq!(names, ["A1", "A2"], "named invoices are re-validated");
        assert_eq!(unknown, ["ZZ9"]);
    }

    #[test]
    fn distinct_affiliates_keep_first_seen_order_and_skip_missing_documents() {
        let owners = vec![
            AffiliateQuery::normalized("CC", "2"),
            AffiliateQuery::normalized("CC", ""),
            AffiliateQuery::normalized("CC", "1"),
            AffiliateQuery::normalized("cc", "2"),
        ];
        let keys: Vec<_> = distinct_affiliates(&owners)
            .iter()
            .map(AffiliateQuery::key)
            .collect();
        assert_eq!(keys, ["CC|2", "CC|1"]);
    }
}
