//! The two rewrites of SISPRO's TXT: the end-of-run sweep of `cuv generar`
//! (`cuv.Sweep`, Java `barrerTxtCuvMinisterioAlFinalizar`) and the folder sweep
//! of `cuv barrer` (`cuv.SweepLocal`, Java `Vista_ActualizarCUVMinisterioLocal`).
//!
//! Both read the TXT as an order-preserving JSON tree, force `ResultState` to
//! `true`, derive `NumFactura` from `RutaArchivos`, fix the RVG18 ("already
//! validated") response and write the tree back with Gson pretty printing and
//! without nulls. Their rules differ (see each function), as in Go.

use super::jsontree::{get, java_trim, parse_ordered, primitive_string, set};
use crate::util::gson::{GsonOptions, Json, to_string};

/// `ConsultarCUV`: the `ProcesoId` of a CUV, if found.
pub type ProcesoIdLookup<'a> = &'a dyn Fn(&str) -> Option<i64>;

/// Result of a sweep of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swept {
    /// The rewritten content; `None` when the file is not a JSON object (Java
    /// skipped it).
    pub out: Option<String>,
    /// The CUV taken from a RVG18 result ("" when none).
    pub cuv: String,
    /// Whether a CUV was taken from a RVG18 result (`SweepLocal` only).
    pub rvg18: bool,
}

impl Swept {
    fn skipped() -> Self {
        Swept {
            out: None,
            cuv: String::new(),
            rvg18: false,
        }
    }
}

fn parse_object(content: &[u8]) -> Option<Vec<(String, Json)>> {
    match parse_ordered(content) {
        Ok(Json::Object(members)) => Some(members),
        _ => None,
    }
}

fn render(root: Vec<(String, Json)>) -> String {
    to_string(&Json::Object(root), &GsonOptions::default())
}

/// `cuv.Sweep`, the per-file body of Java's end-of-run sweep:
///
/// * `ResultState` becomes `true` (even for rejections);
/// * `NumFactura` becomes the last segment of `RutaArchivos` (`\` or `/`
///   separated, trimmed) when that is not blank;
/// * a RVG18 result carries the existing CUV in `Observaciones`: it becomes
///   `CodigoUnicoValidacion`, `ProcesoId` comes from `lookup` and
///   `ResultadosValidacion` is emptied.
///
/// `Ok` with `out: None` when the content is not a JSON object; `Err` when a
/// value Java could not read is found (the file must stay as it is).
pub fn sweep(content: &[u8], lookup: Option<ProcesoIdLookup<'_>>) -> Result<Swept, String> {
    let Some(mut root) = parse_object(content) else {
        return Ok(Swept::skipped());
    };
    set(&mut root, "ResultState", Json::Bool(true));
    if let Some(ruta) = get(&root, "RutaArchivos").filter(|v| **v != Json::Null) {
        let ruta = primitive_string(ruta).map_err(|e| format!("cuv: RutaArchivos: {e}"))?;
        let last = java_trim(last_segment(&ruta)).to_owned();
        if !last.is_empty() {
            set(&mut root, "NumFactura", Json::String(last));
        }
    }
    let cuv = cuv_from_rvg18(&root)?;
    if !cuv.is_empty() {
        apply_existing_cuv(&mut root, &cuv, lookup);
    }
    Ok(Swept {
        out: Some(render(root)),
        rvg18: !cuv.is_empty(),
        cuv,
    })
}

fn apply_existing_cuv(
    root: &mut Vec<(String, Json)>,
    cuv: &str,
    lookup: Option<ProcesoIdLookup<'_>>,
) {
    set(root, "CodigoUnicoValidacion", Json::String(cuv.to_owned()));
    if let Some(id) = lookup.and_then(|find| find(cuv)) {
        set(root, "ProcesoId", Json::Int(id));
    }
    set(root, "ResultadosValidacion", Json::Array(Vec::new()));
}

/// Last element of Java's `ruta.split("[\\\\/]")` (trailing empty strings
/// removed; "" when nothing is left).
fn last_segment(path: &str) -> &str {
    path.trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
}

/// `cuvDesdeRvg18`: the trimmed `Observaciones` of the first RVG18 result that
/// has one, or "".
fn cuv_from_rvg18(root: &[(String, Json)]) -> Result<String, String> {
    let items = match get(root, "ResultadosValidacion") {
        None | Some(Json::Null) => return Ok(String::new()),
        Some(Json::Array(items)) => items,
        Some(_) => return Err("cuv: ResultadosValidacion no es una lista".into()),
    };
    for item in items {
        let Json::Object(result) = item else { continue };
        let Some(code) = get(result, "Codigo").filter(|v| **v != Json::Null) else {
            continue;
        };
        let code = primitive_string(code)?;
        if java_trim(&code) != "RVG18" {
            continue;
        }
        if let Some(obs) = get(result, "Observaciones").filter(|v| **v != Json::Null) {
            return Ok(java_trim(&primitive_string(obs)?).to_owned());
        }
    }
    Ok(String::new())
}

/// `cuv.SweepLocal`, the screen `Vista_ActualizarCUVMinisterioLocal`:
///
/// * `NumFactura` is the last `\`-separated part of `RutaArchivos` (no trim,
///   `/` is not a separator, "" gives "");
/// * the first RVG18 result stops the search; its trimmed `Observaciones`, even
///   "", becomes `CodigoUnicoValidacion` (a null one: no CUV);
/// * a value Java could not read (a non-object result, a non-list or null
///   `ResultadosValidacion`, a path made only of `\`) is an error: Java printed
///   the stack trace and did not rewrite the file.
pub fn sweep_local(content: &[u8], lookup: Option<ProcesoIdLookup<'_>>) -> Result<Swept, String> {
    let Some(mut root) = parse_object(content) else {
        return Ok(Swept::skipped());
    };
    set(&mut root, "ResultState", Json::Bool(true));
    if let Some(ruta) = get(&root, "RutaArchivos").filter(|v| **v != Json::Null) {
        let ruta = primitive_string(ruta).map_err(|e| format!("cuv: RutaArchivos: {e}"))?;
        let name = local_file_name(&ruta)?;
        set(&mut root, "NumFactura", Json::String(name));
    }
    let (cuv, rvg18) = local_rvg18(&root)?;
    if rvg18 {
        apply_existing_cuv(&mut root, &cuv, lookup);
    }
    Ok(Swept {
        out: Some(render(root)),
        cuv,
        rvg18,
    })
}

/// `ruta.split("\\\\")` last part: "" gives "", only separators is an error.
fn local_file_name(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Ok(String::new());
    }
    let trimmed = path.trim_end_matches('\\');
    if trimmed.is_empty() {
        return Err("cuv: RutaArchivos sin nombre de factura".into());
    }
    Ok(trimmed.rsplit('\\').next().unwrap_or("").to_owned())
}

fn local_rvg18(root: &[(String, Json)]) -> Result<(String, bool), String> {
    let Some(value) = get(root, "ResultadosValidacion") else {
        return Ok((String::new(), false));
    };
    let Json::Array(items) = value else {
        return Err("cuv: ResultadosValidacion no es una lista".into());
    };
    for item in items {
        let Json::Object(result) = item else {
            return Err("cuv: ResultadosValidacion con un elemento que no es objeto".into());
        };
        let Some(code) = get(result, "Codigo").filter(|v| **v != Json::Null) else {
            continue;
        };
        if java_trim(&primitive_string(code)?) != "RVG18" {
            continue;
        }
        return match get(result, "Observaciones").filter(|v| **v != Json::Null) {
            Some(obs) => Ok((java_trim(&primitive_string(obs)?).to_owned(), true)),
            None => Ok((String::new(), false)),
        };
    }
    Ok((String::new(), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_segment_follows_java_split() {
        assert_eq!(last_segment(r"\\srv\rips\FVS1"), "FVS1");
        assert_eq!(last_segment(r"C:\Temp\FVS2\"), "FVS2");
        assert_eq!(last_segment("a/b\\c/"), "c");
        assert_eq!(last_segment("FVS3"), "FVS3");
        assert_eq!(last_segment(""), "");
        assert_eq!(last_segment("\\\\"), "");
    }

    #[test]
    fn local_file_name_only_splits_on_backslash() {
        assert_eq!(local_file_name(r"C:\x\FVS1\").expect("n"), "FVS1");
        assert_eq!(local_file_name("a/b/FVS1").expect("n"), "a/b/FVS1");
        assert_eq!(local_file_name("").expect("n"), "");
        assert!(local_file_name(r"\\\").is_err());
    }

    #[test]
    fn sweep_forces_result_state_and_keeps_order_and_literals() {
        let input = br#"{"Zeta":1.50,"ResultState":false,"NumFactura":"X","RutaArchivos":"C:\\a\\FVS9 ","Nulo":null,"ResultadosValidacion":[null,{"Clase":"A"}]}"#;
        let swept = sweep(input, None).expect("sweep");
        assert_eq!(
            swept.out.as_deref(),
            Some(
                "{\n  \"Zeta\": 1.50,\n  \"ResultState\": true,\n  \"NumFactura\": \"FVS9\",\n  \"RutaArchivos\": \"C:\\\\a\\\\FVS9 \",\n  \"ResultadosValidacion\": [\n    null,\n    {\n      \"Clase\": \"A\"\n    }\n  ]\n}"
            )
        );
        assert!(swept.cuv.is_empty() && !swept.rvg18);
    }

    #[test]
    fn sweep_leaves_non_objects_alone_and_rejects_unreadable_values() {
        for content in [&b"plain text"[..], b"[1]", b"", b"{\"a\":1} x"] {
            let swept = sweep(content, None).expect("skip");
            assert!(
                swept.out.is_none(),
                "{:?}",
                String::from_utf8_lossy(content)
            );
        }
        assert!(sweep(br#"{"RutaArchivos":{}}"#, None).is_err());
        assert!(sweep(br#"{"ResultadosValidacion":{}}"#, None).is_err());
        assert!(sweep(br#"{"ResultadosValidacion":[{"Codigo":[]}]}"#, None).is_err());
        // A blank last segment keeps the NumFactura as it was.
        let kept = sweep(br#"{"NumFactura":"K","RutaArchivos":" \\ "}"#, None).expect("blank");
        assert!(kept.out.expect("out").contains("\"NumFactura\": \"K\""));
    }

    #[test]
    fn sweep_rvg18_takes_the_cuv_and_asks_for_the_proceso_id() {
        let input = br#"{"ResultState":false,"ProcesoId":0,"ResultadosValidacion":[{"Clase":"RECHAZADO","Codigo":" RVG18 ","Observaciones":null},{"Clase":"RECHAZADO","Codigo":"RVG18","Observaciones":"  abc123 "},{"Codigo":"RVG18","Observaciones":"later"}]}"#;
        let asked = std::cell::RefCell::new(Vec::new());
        let lookup = |cuv: &str| {
            asked.borrow_mut().push(cuv.to_owned());
            Some(4242)
        };
        let swept = sweep(input, Some(&lookup)).expect("sweep");
        assert_eq!(swept.cuv, "abc123");
        assert_eq!(asked.into_inner(), ["abc123"]);
        assert_eq!(
            swept.out.as_deref(),
            Some(
                "{\n  \"ResultState\": true,\n  \"ProcesoId\": 4242,\n  \"ResultadosValidacion\": [],\n  \"CodigoUnicoValidacion\": \"abc123\"\n}"
            )
        );
        // A failed lookup keeps the ProcesoId it had.
        let none = |_: &str| None;
        let swept = sweep(input, Some(&none)).expect("sweep");
        assert!(swept.out.expect("out").contains("\"ProcesoId\": 0"));
    }

    #[test]
    fn sweep_local_has_its_own_rules() {
        // The first RVG18 stops the search even without Observaciones.
        let input = br#"{"RutaArchivos":"C:\\x\\FVS1\\","ResultadosValidacion":[{"Codigo":"RVG18"},{"Codigo":"RVG18","Observaciones":"late"}]}"#;
        let swept = sweep_local(input, None).expect("local");
        assert!(!swept.rvg18 && swept.cuv.is_empty());
        assert!(swept.out.expect("out").contains("\"NumFactura\": \"FVS1\""));
        // An empty Observaciones is a RVG18 fix with an empty CUV.
        let input = br#"{"ResultadosValidacion":[{"Codigo":"RVG18","Observaciones":" "}]}"#;
        let swept = sweep_local(input, None).expect("local");
        assert!(swept.rvg18 && swept.cuv.is_empty());
        assert!(
            swept
                .out
                .expect("out")
                .contains("\"CodigoUnicoValidacion\": \"\"")
        );
        // A null ResultadosValidacion is an error here (not in `sweep`).
        assert!(sweep_local(br#"{"ResultadosValidacion":null}"#, None).is_err());
        assert!(sweep_local(br#"{"ResultadosValidacion":[1]}"#, None).is_err());
        assert!(sweep(br#"{"ResultadosValidacion":null}"#, None).is_ok());
        assert!(sweep_local(b"nope", None).expect("skip").out.is_none());
    }
}
