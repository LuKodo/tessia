//! Maps a prepared invoice and its validated affiliate to the RIPS document,
//! following `EnvioRIPS_Financiera_Extra` (`cruce_usuarios`,
//! `cruce_medicamentos`, `ImprimirJSON`). Go source: `internal/rips/build.go`.

use anyhow::{Result, bail};

use super::derechos::AffiliateRights;
use super::encode::normalize_id_mipres;
use super::input::{RipsInput, RipsLine};
use super::java::{
    birth_date_iso, extract_hour_minute, go_quote, go_upper, java_double_to_int, java_money,
    java_parse_double, java_parse_int, java_trim, normalize_fecha_xml, normalize_tipo_usuario,
    normalize_zona, truncate_utf16,
};
use super::model::{Medicamento, OtroServicio, Rips, Servicios, Usuario};

// Constants hardcoded in EnvioRIPS_Financiera_Extra.
const TIPO_MEDICAMENTO: &str = "01";
const TIPO_OS: &str = "01";
const DIAS_TRATAMIENTO: i32 = 30;
const TIPO_DOC_MEDICO: &str = "CC";
const DOC_MEDICO_DEFAULT: &str = "11111111";
const COD_PAIS: &str = "031";
const INCAPACIDAD: &str = "NO";
const MAX_NOMBRE_TECNO: usize = 30;
const PRODUCT_MEDICINE: &str = "714";
const PRODUCT_OTHER: &str = "799";
/// Medicines with this concentration are left out.
const CONCENTRACION_UPR: &str = "UPR";
const USER_TYPE_CONTRIBUTIVO: &str = "01";

/// Skip code: no medicine (714, not UPR) and no other service (799).
pub const CODE_NO_SERVICES: &str = "SIN_SERVICIOS";
/// Skip code: a value Java would fail to parse.
pub const CODE_INVALID_DATA: &str = "DATOS_INVALIDOS";
/// Skip code: a line's affiliate differs from the validated one.
pub const CODE_AFFILIATE_MISMATCH: &str = "AFILIADO_NO_COINCIDE";
/// Skip code: the invoice has no `DONE` rights.
pub const CODE_RIGHTS_NOT_OK: &str = "DERECHOS_NO_OK";
/// Skip code: the invoice is excluded or in manual review in the convenio.
pub const CODE_CONVENIO_NOT_OK: &str = "CONVENIO_NO_OK";

/// Why an invoice gets no RIPS JSON. `detail` never carries patient document
/// numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipError {
    pub code: &'static str,
    pub detail: String,
}

impl SkipError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// Returns the `"HH:mm"` used when a dispensing date has no time. `kind` is
/// `"medicamento"` or `"otroServicio"`; `seq` is the item's consecutivo.
pub type HourFn = fn(invoice: &str, kind: &str, seq: usize) -> String;

/// The values the builder does not find in the invoice or the rights.
#[derive(Debug, Clone)]
pub struct RipsOptions {
    /// `numDocumentoIdObligado` (`company.nit_emisor`).
    pub nit: String,
    /// REPS code with sede, 12 digits.
    pub cod_prestador: String,
    /// Replaces Java's random 07:00-16:59 time.
    pub hour: HourFn,
}

impl RipsOptions {
    pub fn new(nit: &str, cod_prestador: &str) -> Self {
        Self {
            nit: nit.to_owned(),
            cod_prestador: cod_prestador.to_owned(),
            hour: deterministic_hour,
        }
    }
}

/// A time in Java's random range (hour 7..16, any minute) from an FNV-32a
/// hash of `invoice|kind|seq`, so regenerating a file gives the same bytes.
pub fn deterministic_hour(invoice: &str, kind: &str, seq: usize) -> String {
    let mut hash: u32 = 2_166_136_261;
    for byte in format!("{invoice}|{kind}|{seq}").bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("{:02}:{:02}", 7 + hash % 10, (hash / 10) % 60)
}

/// Result of [`build`].
pub type BuildResult = Result<std::result::Result<Rips, SkipError>>;

/// Builds the document of one invoice. The inner `Err` is a skip (Java wrote
/// `_SIN_SERVICIOS.txt` / `_JSON_ERROR.txt` instead); the outer `Err` is a
/// programming/configuration error.
pub fn build(input: &RipsInput, aff: &AffiliateRights, opts: &RipsOptions) -> BuildResult {
    if opts.nit.is_empty() || opts.cod_prestador.is_empty() {
        bail!("rips: NIT and CodPrestador are required");
    }
    Ok(build_document(input, aff, opts))
}

fn build_document(
    input: &RipsInput,
    aff: &AffiliateRights,
    opts: &RipsOptions,
) -> Result<Rips, SkipError> {
    let Some(first) = input.lines.first() else {
        return Err(SkipError::new(
            CODE_NO_SERVICES,
            "la factura no tiene lineas en convenio.json",
        ));
    };
    if let Some(line) = input.lines.iter().find(|l| affiliate_key(l) != aff.key()) {
        return Err(SkipError::new(
            CODE_AFFILIATE_MISMATCH,
            format!(
                "la linea {} no es del afiliado validado en derechos.json",
                line.source_line
            ),
        ));
    }
    let tipo_usuario = normalize_tipo_usuario(&aff.rights.user_type);
    let (meds, oss) = build_services(input, &tipo_usuario, opts)?;
    let user = Usuario {
        tipo_documento_identificacion: first.doc_type.clone(),
        num_documento_identificacion: first.doc_number.clone(),
        tipo_usuario,
        fecha_nacimiento: birth_date_iso(&aff.rights.birth_date),
        cod_sexo: aff.rights.sex.clone(),
        cod_pais_residencia: COD_PAIS.to_owned(),
        cod_municipio_residencia: aff.rights.dane.clone(),
        cod_zona_territorial_residencia: normalize_zona(&aff.rights.zone),
        incapacidad: INCAPACIDAD.to_owned(),
        consecutivo: 1,
        cod_pais_origen: COD_PAIS.to_owned(),
        registro_siras: None,
        servicios: Servicios {
            medicamentos: (!meds.is_empty()).then_some(meds),
            otros_servicios: (!oss.is_empty()).then_some(oss),
        },
    };
    Ok(Rips {
        num_documento_id_obligado: opts.nit.clone(),
        num_factura: input.invoice.clone(),
        tipo_nota: None,
        num_nota: None,
        usuarios: vec![user],
    })
}

/// `convenio.AffiliateKey`: upper-cased, trimmed type and number.
fn affiliate_key(line: &RipsLine) -> String {
    format!(
        "{}|{}",
        go_upper(line.doc_type.trim()),
        line.doc_number.trim()
    )
}

type Services = (Vec<Medicamento>, Vec<OtroServicio>);

fn build_services(
    input: &RipsInput,
    tipo_usuario: &str,
    opts: &RipsOptions,
) -> Result<Services, SkipError> {
    let mut meds = Vec::new();
    let mut oss = Vec::new();
    for line in &input.lines {
        let invalid = |detail: String| {
            SkipError::new(
                CODE_INVALID_DATA,
                format!("linea {}: {detail}", line.source_line),
            )
        };
        match line.product_type.as_str() {
            PRODUCT_MEDICINE => {
                let seq = meds.len() + 1;
                if let Some(m) =
                    medicamento(&input.invoice, line, tipo_usuario, seq, opts).map_err(invalid)?
                {
                    meds.push(m);
                }
            }
            PRODUCT_OTHER => {
                let seq = oss.len() + 1;
                oss.push(
                    otro_servicio(&input.invoice, line, tipo_usuario, seq, opts)
                        .map_err(invalid)?,
                );
            }
            _ => {}
        }
    }
    if meds.is_empty() && oss.is_empty() {
        return Err(SkipError::new(
            CODE_NO_SERVICES,
            "la factura no tiene medicamentos (714, sin concentracion UPR) ni otros servicios (799)",
        ));
    }
    Ok((meds, oss))
}

type FieldResult<T> = std::result::Result<T, String>;

/// `None` (no error) for a UPR line, which Java skipped.
fn medicamento(
    invoice: &str,
    l: &RipsLine,
    tipo_usuario: &str,
    seq: usize,
    opts: &RipsOptions,
) -> FieldResult<Option<Medicamento>> {
    let conc = empty_to(&l.concentration, "0");
    if conc == CONCENTRACION_UPR {
        return Ok(None);
    }
    let fecha = dispensing_date_time(&l.date, || (opts.hour)(invoice, "medicamento", seq))?;
    let diag = go_upper(&l.diagnosis_code);
    let mut m = Medicamento {
        cod_prestador: opts.cod_prestador.clone(),
        id_mipres: id_mipres(l),
        fecha_dispens_admon: fecha,
        cod_diagnostico_relacionado: related_diagnosis(&diag),
        cod_diagnostico_principal: diag,
        cod_diagnostico_principal_cie11: None,
        nom_cod_diagnostico_principal_cie11: None,
        cod_diagnostico_relacionado_cie11: None,
        nom_cod_diagnostico_relacionado_cie11: None,
        tipo_medicamento: TIPO_MEDICAMENTO.to_owned(),
        cod_tecnologia_salud: l.cum.clone(),
        nom_tecnologia_salud: truncate_utf16(&l.article, MAX_NOMBRE_TECNO),
        concentracion_medicamento: 0,
        unidad_medida: 0,
        forma_farmaceutica: empty_to(&l.pharmaceutical_form, "NA").to_owned(),
        unidad_min_dispensa: 0,
        cantidad_medicamento: 0,
        dias_tratamiento: DIAS_TRATAMIENTO,
        tipo_documento_identificacion: TIPO_DOC_MEDICO.to_owned(),
        num_documento_identificacion: empty_to(&l.prescriber_id, DOC_MEDICO_DEFAULT).to_owned(),
        vr_unit_medicamento: 0,
        vr_dispensacion: 0,
        vr_servicio: 0,
        concepto_recaudo: None,
        valor_pago_moderador: 0,
        num_fev_pago_moderador: None,
        consecutivo: 0,
        codigo_vida: None,
    };
    m.concentracion_medicamento = int_field("concentracion", conc)?;
    m.unidad_medida = int_field("unidad de medida", empty_to(&l.measure_unit, "0"))?;
    let qty = int_field("cantidad", &l.quantity)?;
    m.unidad_min_dispensa = qty.min(99);
    m.cantidad_medicamento = qty;
    let cuota_peso = double_field("cuota", &l.copay.replace(',', "."))?;
    if is_decimal(&l.unit_value)? {
        // Java truncates with (int) and leaves conceptoRecaudo null here.
        m.vr_unit_medicamento = trunc_field("valor unitario", &l.unit_value)?;
        m.vr_servicio = trunc_field("valor total", &l.total_value)?;
        m.valor_pago_moderador = trunc_field("cuota", &l.copay)?;
    } else {
        m.vr_unit_medicamento = int_field("valor unitario", &java_money(&l.unit_value))?;
        m.vr_servicio = int_field("valor total", &java_money(&l.total_value))?;
        let (concepto, valor) = recaudo(tipo_usuario, cuota_peso, &l.copay)?;
        m.concepto_recaudo = Some(concepto);
        m.valor_pago_moderador = valor;
    }
    m.consecutivo = i32::try_from(seq).unwrap_or(i32::MAX);
    Ok(Some(m))
}

fn otro_servicio(
    invoice: &str,
    l: &RipsLine,
    tipo_usuario: &str,
    seq: usize,
    opts: &RipsOptions,
) -> FieldResult<OtroServicio> {
    let fecha = dispensing_date_time(&l.date, || (opts.hour)(invoice, "otroServicio", seq))?;
    let mut o = OtroServicio {
        cod_prestador: opts.cod_prestador.clone(),
        num_autorizacion: Some(java_trim(&l.authorization).to_owned()),
        id_mipres: id_mipres(l),
        fecha_suministro_tecnologia: fecha,
        tipo_os: TIPO_OS.to_owned(),
        cod_tecnologia_salud: l.cum.clone(),
        nom_tecnologia_salud: truncate_utf16(&l.article, MAX_NOMBRE_TECNO),
        cantidad_os: 0,
        tipo_documento_identificacion: TIPO_DOC_MEDICO.to_owned(),
        num_documento_identificacion: empty_to(&l.prescriber_id, DOC_MEDICO_DEFAULT).to_owned(),
        vr_unit_os: 0,
        vr_dispensacion: 0,
        vr_servicio: 0,
        concepto_recaudo: None,
        valor_pago_moderador: 0,
        num_fev_pago_moderador: None,
        codigo_vida: None,
        consecutivo: i32::try_from(seq).unwrap_or(i32::MAX),
    };
    o.cantidad_os = int_field("cantidad", &l.quantity)?;
    let decimal = is_decimal(&l.unit_value)?;
    let cuota_peso = double_field("cuota", &l.copay.replace(',', "."))?;
    if decimal {
        o.vr_unit_os = trunc_field("valor unitario", &l.unit_value)?;
        o.vr_servicio = trunc_field("valor total", &l.total_value)?;
    } else {
        o.vr_unit_os = int_field("valor unitario", &java_money(&l.unit_value))?;
        o.vr_servicio = int_field("valor total", &java_money(&l.total_value))?;
    }
    let (concepto, valor) = recaudo(tipo_usuario, cuota_peso, &l.copay)?;
    o.concepto_recaudo = Some(concepto);
    o.valor_pago_moderador = valor;
    Ok(o)
}

/// Contributivo (`01`) with a copay pays it as concepto `02`; everyone else is
/// `05` with 0.
fn recaudo(tipo_usuario: &str, cuota_peso: f64, copay: &str) -> FieldResult<(String, i32)> {
    if tipo_usuario == USER_TYPE_CONTRIBUTIVO && cuota_peso > 0.0 {
        let valor = int_field("cuota", &java_money(copay))?;
        Ok(("02".to_owned(), valor))
    } else {
        Ok(("05".to_owned(), 0))
    }
}

/// Java passes the same column (47) as principal and related diagnosis, so
/// the related code is `Z000` for `Z008` and `Z008` otherwise.
fn related_diagnosis(principal: &str) -> String {
    match principal {
        "Z008" => "Z000".to_owned(),
        _ => "Z008".to_owned(),
    }
}

/// Only when the `mipres` column has a value, and then (as in Java) the value
/// sent is the `no. formula` column.
fn id_mipres(l: &RipsLine) -> Option<String> {
    if java_trim(&l.mipres).is_empty() {
        return None;
    }
    normalize_id_mipres(&l.formula_number)
}

/// `obtenerFechaDispensacionDtm` + `normalizarFechaHoraDispensacion`:
/// `"yyyy-MM-dd HH:mm"`, with `hour()` when the value carries no time (always,
/// once the date is normalized).
fn dispensing_date_time(raw: &str, hour: impl FnOnce() -> String) -> FieldResult<String> {
    let mut texto = java_trim(raw).to_owned();
    if texto.is_empty() {
        return Err("fecha de dispensacion vacia".to_owned());
    }
    let normalized = normalize_fecha_xml(&texto);
    if !normalized.is_empty() {
        texto = normalized;
    }
    let mut fecha = normalize_fecha_xml(&texto);
    if fecha.is_empty() {
        fecha = if texto.len() >= 10 {
            truncate_utf16(&texto, 10)
        } else {
            texto.clone()
        };
    }
    let mut h = extract_hour_minute(&texto);
    if h.is_empty() {
        h = hour();
    }
    Ok(format!("{fecha} {h}"))
}

/// `Double.parseDouble(javaMoney(unit)) % 1 != 0`.
fn is_decimal(unit: &str) -> FieldResult<bool> {
    let value = double_field("valor unitario", &java_money(unit))?;
    Ok(value % 1.0 != 0.0)
}

fn trunc_field(name: &str, s: &str) -> FieldResult<i32> {
    double_field(name, &s.replace(',', ".")).map(java_double_to_int)
}

fn int_field(name: &str, s: &str) -> FieldResult<i32> {
    java_parse_int(s).ok_or_else(|| format!("{name} no es un entero: {}", go_quote(s)))
}

fn double_field(name: &str, s: &str) -> FieldResult<f64> {
    java_parse_double(s).ok_or_else(|| format!("{name} no es numerico: {}", go_quote(s)))
}

fn empty_to<'a>(s: &'a str, default: &'a str) -> &'a str {
    if s.is_empty() { default } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phases::rips::encode::encode;

    fn fixed_hour(_: &str, _: &str, _: usize) -> String {
        "08:15".to_owned()
    }

    fn opts() -> RipsOptions {
        RipsOptions {
            hour: fixed_hour,
            ..RipsOptions::new("806011811", "130018500201")
        }
    }

    fn med_line(src: i32) -> RipsLine {
        RipsLine {
            source_line: src,
            doc_type: "CC".into(),
            doc_number: "900000001".into(),
            date: "2026-09-01 10:22:33".into(),
            article: "ACETAMINOFEN (500MG) TABLETA RECUBIERTA X 100".into(),
            quantity: "2".into(),
            unit_value: "1500".into(),
            total_value: "3000".into(),
            copay: "0".into(),
            cum: "19950404-1".into(),
            diagnosis_code: "i10x".into(),
            product_type: "714".into(),
            pharmaceutical_form: "TABLETA".into(),
            concentration: "500".into(),
            measure_unit: "168".into(),
            ..RipsLine::default()
        }
    }

    fn os_line(src: i32) -> RipsLine {
        RipsLine {
            product_type: "799".into(),
            article: "GASA".into(),
            cum: "OS-1".into(),
            authorization: " AUT-9 ".into(),
            concentration: String::new(),
            measure_unit: String::new(),
            pharmaceutical_form: String::new(),
            ..med_line(src)
        }
    }

    fn affiliate(user_type: &str) -> AffiliateRights {
        AffiliateRights {
            doc_type: "CC".into(),
            doc_number: "900000001".into(),
            rights: super::super::derechos::Rights {
                user_type: user_type.into(),
                zone: "U".into(),
                dane: "13001".into(),
                sex: "F".into(),
                birth_date: "1990-01-02".into(),
            },
        }
    }

    fn input(lines: Vec<RipsLine>) -> RipsInput {
        RipsInput {
            invoice: "FVS1".into(),
            lines,
        }
    }

    fn built(user_type: &str, lines: Vec<RipsLine>) -> Rips {
        build(&input(lines), &affiliate(user_type), &opts())
            .expect("build")
            .expect("not skipped")
    }

    fn skipped(lines: Vec<RipsLine>) -> SkipError {
        build(&input(lines), &affiliate("01"), &opts())
            .expect("build")
            .expect_err("skip")
    }

    const END_TO_END: &str = r#"{
  "numDocumentoIdObligado": "806011811",
  "numFactura": "FVS1",
  "tipoNota": null,
  "numNota": null,
  "usuarios": [
    {
      "tipoDocumentoIdentificacion": "CC",
      "numDocumentoIdentificacion": "900000001",
      "tipoUsuario": "01",
      "fechaNacimiento": "1990-01-02",
      "codSexo": "F",
      "codPaisResidencia": "031",
      "codMunicipioResidencia": "13001",
      "codZonaTerritorialResidencia": "02",
      "incapacidad": "NO",
      "consecutivo": 1,
      "codPaisOrigen": "031",
      "registroSIRAS": null,
      "servicios": {
        "medicamentos": [
          {
            "codPrestador": "130018500201",
            "idMIPRES": null,
            "fechaDispensAdmon": "2026-09-01 08:15",
            "codDiagnosticoPrincipal": "I10X",
            "codDiagnosticoPrincipalCIE11": null,
            "nomCodDiagnosticoPrincipalCIE11": null,
            "codDiagnosticoRelacionado": "Z008",
            "codDiagnosticoRelacionadoCIE11": null,
            "nomCodDiagnosticoRelacionadoCIE11": null,
            "tipoMedicamento": "01",
            "codTecnologiaSalud": "19950404-1",
            "nomTecnologiaSalud": "ACETAMINOFEN 500MG TABLETA R",
            "concentracionMedicamento": 500,
            "unidadMedida": 168,
            "formaFarmaceutica": "TABLETA",
            "unidadMinDispensa": 2,
            "cantidadMedicamento": 2,
            "diasTratamiento": 30,
            "tipoDocumentoIdentificacion": "CC",
            "numDocumentoIdentificacion": "11111111",
            "vrUnitMedicamento": 1500,
            "vrDispensacion": 0,
            "vrServicio": 3000,
            "conceptoRecaudo": "05",
            "valorPagoModerador": 0,
            "numFEVPagoModerador": null,
            "consecutivo": 1,
            "codigoVIDA": null
          }
        ],
        "otrosServicios": [
          {
            "codPrestador": "130018500201",
            "numAutorizacion": "AUT-9",
            "idMIPRES": null,
            "fechaSuministroTecnologia": "2026-09-01 08:15",
            "tipoOS": "01",
            "codTecnologiaSalud": "OS-1",
            "nomTecnologiaSalud": "GASA",
            "cantidadOS": 2,
            "tipoDocumentoIdentificacion": "CC",
            "numDocumentoIdentificacion": "11111111",
            "vrUnitOS": 1500,
            "vrDispensacion": 0,
            "vrServicio": 3000,
            "conceptoRecaudo": "05",
            "valorPagoModerador": 0,
            "numFEVPagoModerador": null,
            "codigoVIDA": null,
            "consecutivo": 1
          }
        ]
      }
    }
  ]
}"#;

    #[test]
    fn end_to_end_bytes_match_the_hand_verified_go_document() {
        let rips = built("01", vec![med_line(2), os_line(3)]);
        assert_eq!(encode(&rips).expect("encode"), END_TO_END);
    }

    #[test]
    fn user_fields_are_normalized() {
        let mut aff = affiliate("04");
        aff.rights.zone = "R".into();
        aff.rights.birth_date = "02/01/1990".into();
        aff.rights.sex = "M".into();
        let rips = build(&input(vec![med_line(2)]), &aff, &opts())
            .expect("build")
            .expect("rips");
        let user = &rips.usuarios[0];
        assert_eq!(user.tipo_usuario, "04");
        assert_eq!(user.cod_zona_territorial_residencia, "01");
        assert_eq!(user.fecha_nacimiento, "1990-01-02");
        assert_eq!(user.cod_sexo, "M");
        assert!(user.servicios.otros_servicios.is_none(), "no 799 lines");
    }

    #[test]
    fn upr_lines_are_dropped_and_not_counted_in_consecutivos() {
        let mut upr = med_line(3);
        upr.concentration = "UPR".into();
        let rips = built(
            "01",
            vec![med_line(2), upr, med_line(4), os_line(5), os_line(6)],
        );
        let s = &rips.usuarios[0].servicios;
        let meds = s.medicamentos.as_ref().expect("meds");
        assert_eq!(
            meds.iter().map(|m| m.consecutivo).collect::<Vec<_>>(),
            [1, 2]
        );
        let oss = s.otros_servicios.as_ref().expect("oss");
        assert_eq!(
            oss.iter().map(|o| o.consecutivo).collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn only_upr_lines_is_sin_servicios() {
        let mut upr = med_line(2);
        upr.concentration = "UPR".into();
        let skip = skipped(vec![upr]);
        assert_eq!(skip.code, CODE_NO_SERVICES);
        assert_eq!(
            skip.detail,
            "la factura no tiene medicamentos (714, sin concentracion UPR) ni otros servicios (799)"
        );
        assert_eq!(skipped(vec![]).code, CODE_NO_SERVICES);
    }

    #[test]
    fn recaudo_rules_and_the_java_money_quirks() {
        // (user type, unit, total, copay) -> (concept, unit, total, moderador)
        let cases = [
            ("01", "1500", "3000", "3500", Some("02"), 1500, 3000, 3500),
            ("01", "1500", "3000", "0", Some("05"), 1500, 3000, 0),
            ("04", "1500", "3000", "3500", Some("05"), 1500, 3000, 0),
            // Decimal unit value: (int) truncation and conceptoRecaudo left null.
            ("01", "1500,75", "3001,5", "10,9", None, 1500, 3001, 10),
            ("01", "1500,00", "3000,00", "0", Some("05"), 1500, 3000, 0),
        ];
        for (user, unit, total, copay, concept, want_unit, want_total, want_mod) in cases {
            let mut l = med_line(2);
            l.unit_value = unit.into();
            l.total_value = total.into();
            l.copay = copay.into();
            let rips = built(user, vec![l]);
            let m = &rips.usuarios[0]
                .servicios
                .medicamentos
                .as_ref()
                .expect("meds")[0];
            let label = format!("{user} {unit} {total} {copay}");
            assert_eq!(m.concepto_recaudo.as_deref(), concept, "{label}");
            assert_eq!(m.vr_unit_medicamento, want_unit, "{label}");
            assert_eq!(m.vr_servicio, want_total, "{label}");
            assert_eq!(m.valor_pago_moderador, want_mod, "{label}");
        }
        // Other services always set conceptoRecaudo, also for decimal values.
        let mut l = os_line(2);
        l.unit_value = "10,5".into();
        l.total_value = "21".into();
        l.copay = "700".into();
        let rips = built("01", vec![l]);
        let o = &rips.usuarios[0]
            .servicios
            .otros_servicios
            .as_ref()
            .expect("oss")[0];
        assert_eq!(o.concepto_recaudo.as_deref(), Some("02"));
        assert_eq!(
            (o.vr_unit_os, o.vr_servicio, o.valor_pago_moderador),
            (10, 21, 700)
        );
    }

    #[test]
    fn values_java_cannot_parse_skip_the_invoice_with_the_line_number() {
        type Mutation = fn(&mut RipsLine);
        let mutations: [(Mutation, &str); 6] = [
            (
                |l| l.concentration = "500 MG".into(),
                "concentracion no es un entero: \"500 MG\"",
            ),
            (
                |l| l.measure_unit = "MG".into(),
                "unidad de medida no es un entero: \"MG\"",
            ),
            (
                |l| l.quantity = "1.5".into(),
                "cantidad no es un entero: \"1.5\"",
            ),
            (
                |l| l.unit_value = "abc".into(),
                "valor unitario no es numerico: \"abc\"",
            ),
            // Integer.parseInt("3000.0") throws in Java.
            (
                |l| l.total_value = "3000.0".into(),
                "valor total no es un entero: \"3000.0\"",
            ),
            (|l| l.date = String::new(), "fecha de dispensacion vacia"),
        ];
        for (mutate, detail) in mutations {
            let mut l = med_line(7);
            mutate(&mut l);
            let skip = skipped(vec![l]);
            assert_eq!(skip.code, CODE_INVALID_DATA);
            assert_eq!(skip.detail, format!("linea 7: {detail}"));
        }
    }

    #[test]
    fn medicine_defaults_and_mipres_rule() {
        let mut l = med_line(2);
        l.concentration = String::new();
        l.measure_unit = String::new();
        l.pharmaceutical_form = String::new();
        l.quantity = "150".into();
        l.prescriber_id = "123".into();
        l.mipres = "MIP-1".into();
        l.formula_number = " 20260901 ".into();
        l.diagnosis_code = "Z008".into();
        let rips = built("01", vec![l.clone()]);
        let m = &rips.usuarios[0]
            .servicios
            .medicamentos
            .as_ref()
            .expect("meds")[0];
        assert_eq!(m.concentracion_medicamento, 0);
        assert_eq!(m.unidad_medida, 0);
        assert_eq!(m.forma_farmaceutica, "NA");
        assert_eq!((m.unidad_min_dispensa, m.cantidad_medicamento), (99, 150));
        assert_eq!(m.num_documento_identificacion, "123");
        assert_eq!(m.cod_diagnostico_relacionado, "Z000");
        assert_eq!(m.id_mipres.as_deref(), Some("20260901"));
        // "mipres" empty: null even with a formula number.
        l.mipres = String::new();
        let rips = built("01", vec![l]);
        let m = &rips.usuarios[0]
            .servicios
            .medicamentos
            .as_ref()
            .expect("meds")[0];
        assert_eq!(m.id_mipres, None);
    }

    #[test]
    fn a_line_of_another_affiliate_is_a_mismatch_without_document_numbers() {
        let mut other = med_line(2);
        other.doc_number = "900000009".into();
        let skip = skipped(vec![med_line(1), other]);
        assert_eq!(skip.code, CODE_AFFILIATE_MISMATCH);
        assert_eq!(
            skip.detail,
            "la linea 2 no es del afiliado validado en derechos.json"
        );
        assert!(!skip.detail.contains("900000009"));
    }

    #[test]
    fn nit_and_cod_prestador_are_required() {
        let bare = RipsOptions::new("1", "");
        assert!(build(&input(vec![med_line(1)]), &affiliate("01"), &bare).is_err());
    }

    #[test]
    fn dispensing_date_time_matches_the_go_table() {
        let hour = || "09:30".to_owned();
        for (raw, want) in [
            // Java drops the time before adding one.
            ("2026-09-01 10:22:33", "2026-09-01 09:30"),
            ("01/09/2026", "2026-09-01 09:30"),
            // Not normalizable: first 10 chars + the parsed time.
            ("20260901 7:05", "20260901 7 07:05"),
            ("ayer", "ayer 09:30"),
        ] {
            assert_eq!(
                dispensing_date_time(raw, hour).as_deref(),
                Ok(want),
                "{raw:?}"
            );
        }
        assert!(dispensing_date_time("  ", hour).is_err());
    }

    #[test]
    fn the_deterministic_hour_stays_in_java_range_and_is_stable() {
        let mut seen = std::collections::HashSet::new();
        for seq in 1..=500 {
            let h = deterministic_hour("FVS1", "medicamento", seq);
            assert_eq!(h.len(), 5, "{h}");
            assert!(h.as_str() >= "07:00" && h.as_str() <= "16:59", "{h}");
            assert_eq!(h, deterministic_hour("FVS1", "medicamento", seq));
            seen.insert(h);
        }
        assert!(seen.len() >= 100, "poor spread: {}", seen.len());
    }

    #[test]
    fn the_deterministic_hour_is_fnv32a_of_invoice_kind_seq() {
        // Values computed with an independent FNV-1a 32 implementation (its
        // result for "a" is the published 0xe40c292c): hash 1690170044 maps to
        // hour 7 + 1690170044 % 10 and minute (1690170044 / 10) % 60.
        for (invoice, kind, seq, want) in [
            ("FVS1", "medicamento", 1, "11:04"),
            ("FVS1", "otroServicio", 2, "16:50"),
            ("EVS12", "medicamento", 7, "08:31"),
        ] {
            assert_eq!(deterministic_hour(invoice, kind, seq), want);
        }
    }
}
