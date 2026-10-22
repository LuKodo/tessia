//! The RIPS document (Resolución 2275), mirroring the Java model
//! (`contingenciarips.Modelo.Estructura`) field by field and in the same order.
//! Go source: `internal/rips/model.go`.
//!
//! JSON names are the Java names; nullable Java `String`s are `Option<String>`.
//! Struct field order is the output order (see [`super::encode`]).

use serde::{Deserialize, Serialize};

/// Top-level document written to `<F>.json` (`jsonAlterno` in Java).
/// `xmlFevFile` is not part of the local file: Java only added it to the
/// payload sent to the Ministry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rips {
    #[serde(rename = "numDocumentoIdObligado")]
    pub num_documento_id_obligado: String,
    #[serde(rename = "numFactura")]
    pub num_factura: String,
    #[serde(rename = "tipoNota")]
    pub tipo_nota: Option<String>,
    #[serde(rename = "numNota")]
    pub num_nota: Option<String>,
    pub usuarios: Vec<Usuario>,
}

/// One patient of the invoice (`Usuarios.java`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usuario {
    #[serde(rename = "tipoDocumentoIdentificacion")]
    pub tipo_documento_identificacion: String,
    #[serde(rename = "numDocumentoIdentificacion")]
    pub num_documento_identificacion: String,
    #[serde(rename = "tipoUsuario")]
    pub tipo_usuario: String,
    #[serde(rename = "fechaNacimiento")]
    pub fecha_nacimiento: String,
    #[serde(rename = "codSexo")]
    pub cod_sexo: String,
    #[serde(rename = "codPaisResidencia")]
    pub cod_pais_residencia: String,
    #[serde(rename = "codMunicipioResidencia")]
    pub cod_municipio_residencia: String,
    #[serde(rename = "codZonaTerritorialResidencia")]
    pub cod_zona_territorial_residencia: String,
    pub incapacidad: String,
    pub consecutivo: i32,
    #[serde(rename = "codPaisOrigen")]
    pub cod_pais_origen: String,
    #[serde(rename = "registroSIRAS")]
    pub registro_siras: Option<String>,
    /// Always present: Java only built users that had services.
    #[serde(default)]
    pub servicios: Servicios,
}

/// Service lists of a user. A `None` list is omitted from the output (Java
/// only put non-null lists in the `servicios` map); `medicamentos` first. An
/// empty list is written as `[]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Servicios {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medicamentos: Option<Vec<Medicamento>>,
    #[serde(
        default,
        rename = "otrosServicios",
        skip_serializing_if = "Option::is_none"
    )]
    pub otros_servicios: Option<Vec<OtroServicio>>,
}

/// `Medicamento.java` minus the transient `numAutorizacion`. Note that
/// `consecutivo` comes before `codigoVIDA` here and after it in
/// [`OtroServicio`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Medicamento {
    #[serde(rename = "codPrestador")]
    pub cod_prestador: String,
    #[serde(rename = "idMIPRES")]
    pub id_mipres: Option<String>,
    #[serde(rename = "fechaDispensAdmon")]
    pub fecha_dispens_admon: String,
    #[serde(rename = "codDiagnosticoPrincipal")]
    pub cod_diagnostico_principal: String,
    #[serde(rename = "codDiagnosticoPrincipalCIE11")]
    pub cod_diagnostico_principal_cie11: Option<String>,
    #[serde(rename = "nomCodDiagnosticoPrincipalCIE11")]
    pub nom_cod_diagnostico_principal_cie11: Option<String>,
    #[serde(rename = "codDiagnosticoRelacionado")]
    pub cod_diagnostico_relacionado: String,
    #[serde(rename = "codDiagnosticoRelacionadoCIE11")]
    pub cod_diagnostico_relacionado_cie11: Option<String>,
    #[serde(rename = "nomCodDiagnosticoRelacionadoCIE11")]
    pub nom_cod_diagnostico_relacionado_cie11: Option<String>,
    #[serde(rename = "tipoMedicamento")]
    pub tipo_medicamento: String,
    #[serde(rename = "codTecnologiaSalud")]
    pub cod_tecnologia_salud: String,
    #[serde(rename = "nomTecnologiaSalud")]
    pub nom_tecnologia_salud: String,
    #[serde(rename = "concentracionMedicamento")]
    pub concentracion_medicamento: i32,
    #[serde(rename = "unidadMedida")]
    pub unidad_medida: i32,
    #[serde(rename = "formaFarmaceutica")]
    pub forma_farmaceutica: String,
    #[serde(rename = "unidadMinDispensa")]
    pub unidad_min_dispensa: i32,
    #[serde(rename = "cantidadMedicamento")]
    pub cantidad_medicamento: i32,
    #[serde(rename = "diasTratamiento")]
    pub dias_tratamiento: i32,
    #[serde(rename = "tipoDocumentoIdentificacion")]
    pub tipo_documento_identificacion: String,
    #[serde(rename = "numDocumentoIdentificacion")]
    pub num_documento_identificacion: String,
    #[serde(rename = "vrUnitMedicamento")]
    pub vr_unit_medicamento: i32,
    #[serde(rename = "vrDispensacion")]
    pub vr_dispensacion: i32,
    #[serde(rename = "vrServicio")]
    pub vr_servicio: i32,
    #[serde(rename = "conceptoRecaudo")]
    pub concepto_recaudo: Option<String>,
    #[serde(rename = "valorPagoModerador")]
    pub valor_pago_moderador: i32,
    #[serde(rename = "numFEVPagoModerador")]
    pub num_fev_pago_moderador: Option<String>,
    pub consecutivo: i32,
    #[serde(rename = "codigoVIDA")]
    pub codigo_vida: Option<String>,
}

/// `OtroServicio.java`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtroServicio {
    #[serde(rename = "codPrestador")]
    pub cod_prestador: String,
    #[serde(rename = "numAutorizacion")]
    pub num_autorizacion: Option<String>,
    #[serde(rename = "idMIPRES")]
    pub id_mipres: Option<String>,
    #[serde(rename = "fechaSuministroTecnologia")]
    pub fecha_suministro_tecnologia: String,
    #[serde(rename = "tipoOS")]
    pub tipo_os: String,
    #[serde(rename = "codTecnologiaSalud")]
    pub cod_tecnologia_salud: String,
    #[serde(rename = "nomTecnologiaSalud")]
    pub nom_tecnologia_salud: String,
    #[serde(rename = "cantidadOS")]
    pub cantidad_os: i32,
    #[serde(rename = "tipoDocumentoIdentificacion")]
    pub tipo_documento_identificacion: String,
    #[serde(rename = "numDocumentoIdentificacion")]
    pub num_documento_identificacion: String,
    #[serde(rename = "vrUnitOS")]
    pub vr_unit_os: i32,
    #[serde(rename = "vrDispensacion")]
    pub vr_dispensacion: i32,
    #[serde(rename = "vrServicio")]
    pub vr_servicio: i32,
    #[serde(rename = "conceptoRecaudo")]
    pub concepto_recaudo: Option<String>,
    #[serde(rename = "valorPagoModerador")]
    pub valor_pago_moderador: i32,
    #[serde(rename = "numFEVPagoModerador")]
    pub num_fev_pago_moderador: Option<String>,
    #[serde(rename = "codigoVIDA")]
    pub codigo_vida: Option<String>,
    pub consecutivo: i32,
}
