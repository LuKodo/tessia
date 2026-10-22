//! End-to-end checks of the `facturacion` binary.

use std::process::{Command, Output};

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_facturacion"));
    command.args(args).env_clear();
    // Keep the loader's own variables out of the way; pass only what the test says.
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("run facturacion")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_lists_every_ported_command_group() {
    let out = run(&["--help"], &[]);
    assert!(out.status.success());
    let help = text(&out.stdout);
    for command in [
        "convenio",
        "proceso",
        "conciliar",
        "thefactory",
        "derechos",
        "fev",
        "rips",
        "cuv",
        "soporte",
        "radicacion",
        "config",
        "db",
    ] {
        assert!(
            help.lines().any(|l| l.trim_start().starts_with(command)),
            "{command} missing in:\n{help}"
        );
    }
    for removed in ["serve", "siesa", "seguimiento", "importar"] {
        assert!(!help.contains(removed), "{removed} must not exist");
    }
}

#[test]
fn usage_errors_exit_with_status_2() {
    let out = run(&["rips", "generar", "--dry-run"], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("--dry-run"));
}

#[test]
fn config_check_lists_keys_without_values_and_fails_for_missing_required_ones() {
    let env = [("TESSIA_CASSANDRA_API_KEY", "super-secret-value")];
    let out = run(&["config", "check"], &env);
    assert!(out.status.success());
    let listing = text(&out.stdout);
    assert!(listing.contains("cassandra.api_key") && listing.contains("TESSIA_CASSANDRA_API_KEY"));
    assert!(
        !listing.contains("super-secret-value"),
        "values are never printed"
    );

    let ok = run(&["config", "check", "cassandra.api_key"], &env);
    assert!(ok.status.success());
    assert!(text(&ok.stdout).contains("las 1 claves requeridas están definidas"));

    let missing = run(&["config", "check", "database.url", "redis.url"], &env);
    assert_eq!(missing.status.code(), Some(1));
    let err = text(&missing.stderr);
    assert!(
        err.starts_with("Error: faltan claves de configuración requeridas"),
        "{err}"
    );
    assert!(
        err.contains("database.url (TESSIA_DATABASE_URL)")
            && err.contains("redis.url (TESSIA_REDIS_URL)")
    );
}

#[test]
fn config_file_is_read_and_unknown_keys_are_rejected() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("t.env");
    std::fs::write(&file, "TESSIA_REDIS_URL=redis://example\n").expect("write");
    let out = run(
        &[
            "--config",
            file.to_str().expect("utf8"),
            "config",
            "check",
            "redis.url",
        ],
        &[],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    std::fs::write(&file, "TESSIA_NOPE=1\n").expect("write");
    let out = run(
        &["--config", file.to_str().expect("utf8"), "config", "check"],
        &[],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("TESSIA_NOPE"));
}

#[test]
fn db_migrate_applies_once_and_is_then_a_no_op() {
    use diesel::{Connection, PgConnection, RunQueryDsl, sql_query};

    let admin_url = std::env::var("TESSIA_TEST_PG_ADMIN_URL")
        .unwrap_or_else(|_| tessia::db::test_support::DEFAULT_ADMIN_URL.to_owned());
    let name = format!("tessia_cli_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::establish(&admin_url).expect("admin connection");
    sql_query(format!("CREATE DATABASE \"{name}\""))
        .execute(&mut admin)
        .expect("create database");
    let mut url = url::Url::parse(&admin_url).expect("admin url");
    url.set_path(&format!("/{name}"));
    let env = [("TESSIA_DATABASE_URL", url.as_str())];

    let first = run(&["db", "migrate"], &env);
    let second = run(&["db", "migrate"], &env);
    sql_query(format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
        .execute(&mut admin)
        .expect("drop database");

    assert!(first.status.success(), "{}", text(&first.stderr));
    assert!(
        text(&first.stdout).contains("migraciones aplicadas:"),
        "{}",
        text(&first.stdout)
    );
    assert!(second.status.success(), "{}", text(&second.stderr));
    assert!(text(&second.stdout).contains("ya está al día"));
}

#[test]
fn db_migrate_without_a_database_url_says_what_is_missing() {
    let out = run(&["db", "migrate"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("TESSIA_DATABASE_URL"));
}

#[test]
fn phase_commands_require_the_infrastructure_settings() {
    let out = run(&["rips", "generar", "--cargue", "C1"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("TESSIA_DATABASE_URL"));
}
