//! `cuv barrer`: the folder sweep of the CUV TXTs of a cargue.

#[path = "cuv_support.rs"]
mod support;

use std::collections::HashMap;

use support::*;
use tessia::db::test_support::TestDb;
use tessia::domain::{Outcome, Phase, PhaseState};
use tessia::phases::cuv::{barrer, lease_name};

fn local(name: &str) -> Vec<u8> {
    read(&format!("local/{name}"))
}

#[test]
fn rewrites_the_txts_with_the_screen_rules_and_records_the_cuvs() {
    let db = TestDb::new();
    let fake = FakeSispro::start(HashMap::new());
    let ctx = context(&db, fake.url(), "CT-1");
    let cuv_txt = br#"{"ResultState":false,"CodigoUnicoValidacion":"abc","RutaArchivos":"C:\\a\\FVS2000004"}"#;
    publish(&ctx, "FVS2000002.txt", &local("rvg18.txt"));
    publish(&ctx, "FVS2000003.txt", &local("sin_rvg18.txt"));
    publish(&ctx, "FVS2000004.txt", cuv_txt);
    publish(&ctx, "FVS2000005.txt", &local("no_objeto.txt"));
    publish(&ctx, "FVS2000006.txt", &local("elemento_no_objeto.txt"));
    publish(&ctx, "FVS2000007_ERROR.txt", &local("rvg18.txt"));
    publish(&ctx, "FVS2000008.txt", &local("rvg18.out.txt"));
    publish(&ctx, "CPSCAN/FVS2000009.txt", &local("rvg18.txt"));
    publish(&ctx, "FVS2000010.json", b"{}");
    save_state(
        &ctx,
        Phase::Cuv,
        "FVS2000002",
        Outcome::manual_review("rechazada"),
    );

    let run = barrer::execute(&ctx, CARGUE).expect("barrer");

    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(
        run.lines,
        [
            "  FVS2000002.txt actualizar (RVG18: CUV existente, ProcesoId por ConsultarCUV)",
            "  FVS2000003.txt actualizar",
            "  FVS2000004.txt actualizar",
            "  FVS2000005.txt omitido (no es un JSON)",
            "  FVS2000006.txt ERROR cuv: ResultadosValidacion con un elemento que no es objeto",
            "  FVS2000007_ERROR.txt actualizar (RVG18: CUV existente, ProcesoId por ConsultarCUV)",
            "cuv barrer: actualizar=4 rvg18=2 sin_cambios=1 omitidos=1 errores=1",
            "  FVS2000002.txt actualizado",
            "  FVS2000003.txt actualizado",
            "  FVS2000004.txt actualizado",
            "warning: skipping FVS2000007_ERROR.txt because phase ownership is unavailable",
        ]
    );
    // Rewritten like Vista_ActualizarCUVMinisterioLocal (ProcesoId from ConsultarCUV).
    assert_eq!(fetch(&ctx, "FVS2000002.txt"), local("rvg18.out.txt"));
    assert_eq!(fetch(&ctx, "FVS2000003.txt"), local("sin_rvg18.out.txt"));
    assert_eq!(
        String::from_utf8(fetch(&ctx, "FVS2000004.txt")).expect("utf8"),
        "{\n  \"ResultState\": true,\n  \"CodigoUnicoValidacion\": \"abc\",\n  \"RutaArchivos\": \"C:\\\\a\\\\FVS2000004\",\n  \"NumFactura\": \"FVS2000004\"\n}"
    );
    // Untouched: not JSON, not processable, `_ERROR` files (no invoice to own), up to date.
    assert_eq!(fetch(&ctx, "FVS2000005.txt"), local("no_objeto.txt"));
    assert_eq!(
        fetch(&ctx, "FVS2000006.txt"),
        local("elemento_no_objeto.txt")
    );
    assert_eq!(fetch(&ctx, "FVS2000007_ERROR.txt"), local("rvg18.txt"));
    assert_eq!(fetch(&ctx, "FVS2000008.txt"), local("rvg18.out.txt"));
    assert_eq!(fetch(&ctx, "CPSCAN/FVS2000009.txt"), local("rvg18.txt"));
    assert_eq!(
        fake.count(CONSULTA),
        1,
        "one ConsultarCUV, for the claimed RVG18 file"
    );
    // A rewritten TXT that now carries a CUV is DONE; the RVG18 ones say why.
    let rvg18 = state(&ctx, Phase::Cuv, "FVS2000002").expect("row");
    assert_eq!(
        (rvg18.state, rvg18.motivo.as_str()),
        (PhaseState::Done, "RVG18: CUV existente")
    );
    assert_eq!(rvg18.intentos, 2, "attempts continue from the stored row");
    let plain = state(&ctx, Phase::Cuv, "FVS2000004").expect("row");
    assert_eq!((plain.state, plain.motivo.as_str()), (PhaseState::Done, ""));
    assert!(
        state(&ctx, Phase::Cuv, "FVS2000003").is_none(),
        "no CUV: nothing recorded"
    );
    assert!(state(&ctx, Phase::Cuv, "FVS2000005").is_none());
    let outcomes: Vec<_> = run
        .report
        .outcomes
        .iter()
        .map(|o| o.invoice.as_str())
        .collect();
    assert_eq!(outcomes, ["FVS2000002", "FVS2000004"]);
}

#[test]
fn a_second_sweep_has_nothing_left_to_do() {
    let db = TestDb::new();
    let fake = FakeSispro::start(HashMap::new());
    let ctx = context(&db, fake.url(), "CT-1");
    publish(&ctx, "FVS2000002.txt", &local("rvg18.txt"));
    barrer::execute(&ctx, CARGUE).expect("first");
    let again = barrer::execute(&ctx, CARGUE).expect("second");
    assert_eq!(
        again.lines,
        ["cuv barrer: actualizar=0 rvg18=0 sin_cambios=1 omitidos=0 errores=0"]
    );
    assert_eq!(fake.count(CONSULTA), 1);
}

#[test]
fn credentials_are_only_needed_when_a_file_has_rvg18() {
    let db = TestDb::new();
    let ctx = context(&db, "", "CT-1");
    publish(&ctx, "FVS2000003.txt", &local("sin_rvg18.txt"));
    let run = barrer::execute(&ctx, CARGUE).expect("no sispro needed");
    assert!(run.errors.is_empty());
    assert_eq!(fetch(&ctx, "FVS2000003.txt"), local("sin_rvg18.out.txt"));
    publish(&ctx, "FVS2000002.txt", &local("rvg18.txt"));
    let err = barrer::execute(&ctx, CARGUE).expect_err("needs minsalud.*");
    assert!(err.to_string().contains("minsalud.base_url"), "{err}");
    assert_eq!(
        fetch(&ctx, "FVS2000002.txt"),
        local("rvg18.txt"),
        "nothing was rewritten"
    );
    assert!(
        state(&ctx, Phase::Cuv, "FVS2000002").is_none(),
        "and nothing was claimed"
    );
}

#[test]
fn a_failed_consultar_cuv_keeps_the_proceso_id_it_had() {
    let db = TestDb::new();
    let fake = FakeSispro::start_with(HashMap::new(), false);
    let ctx = context(&db, fake.url(), "CT-1");
    publish(&ctx, "FVS2000002.txt", &local("rvg18.txt"));
    barrer::execute(&ctx, CARGUE).expect("barrer");
    let out = String::from_utf8(fetch(&ctx, "FVS2000002.txt")).expect("utf8");
    assert!(out.contains("\"ProcesoId\": 0"), "{out}");
    assert!(out.contains("\"CodigoUnicoValidacion\": \"1dddd"), "{out}");
    assert_eq!(
        state(&ctx, Phase::Cuv, "FVS2000002").expect("row").motivo,
        "RVG18: CUV existente"
    );
}

#[test]
fn an_invoice_another_worker_holds_is_skipped_and_its_state_restored() {
    let db = TestDb::new();
    let fake = FakeSispro::start(HashMap::new());
    let ctx = context(&db, fake.url(), "CT-1");
    let other = ctx.clone().with_owner("fev-ajustar:1:xyz");
    publish(&ctx, "FVS2000002.txt", &local("rvg18.txt"));
    publish(&ctx, "FVS2000003.txt", &local("sin_rvg18.txt"));
    save_state(
        &ctx,
        Phase::Cuv,
        "FVS2000002",
        Outcome::manual_review("rechazada"),
    );
    assert!(
        other
            .leases()
            .acquire(
                &lease_name(CARGUE, "FVS2000002"),
                &other.owner,
                std::time::Duration::from_secs(600)
            )
            .expect("lease")
    );

    let run = barrer::execute(&ctx, CARGUE).expect("barrer");

    assert!(run.lines.contains(
        &"warning: skipping FVS2000002.txt because phase ownership is unavailable".to_owned()
    ));
    assert_eq!(
        fetch(&ctx, "FVS2000002.txt"),
        local("rvg18.txt"),
        "left as it was"
    );
    assert_eq!(
        fetch(&ctx, "FVS2000003.txt"),
        local("sin_rvg18.out.txt"),
        "the others still go"
    );
    let row = state(&ctx, Phase::Cuv, "FVS2000002").expect("row");
    assert_eq!(
        (row.state, row.motivo.as_str()),
        (PhaseState::RevisionManual, "rechazada")
    );
    assert_eq!(row.claimed_by, None, "the claim was given back");
    assert_eq!(fake.count(CONSULTA), 0);
}
