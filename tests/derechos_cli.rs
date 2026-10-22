//! `facturacion derechos validar` as the user sees it.

use std::process::Command;

fn help(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_facturacion"))
        .args(args)
        .env_clear()
        .output()
        .expect("run facturacion");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn validar_help_documents_every_flag_in_spanish() {
    let text = help(&["derechos", "validar", "--help"]);
    for needle in [
        "--cargue",
        "--invoices",
        "--concurrency",
        "--cache-ttl",
        "[default: 30]",
        "[default: 24h]",
        "REVISION_MANUAL",
        "formato preparado",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
    assert!(!text.contains("--live") && !text.contains("--dry-run"));
}

#[test]
fn the_group_lists_validar() {
    let text = help(&["derechos", "--help"]);
    assert!(text.contains("validar"), "{text}");
}
