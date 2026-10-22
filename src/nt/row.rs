//! A row of the Nota Técnica.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One NT row as stored (JSON) in `<p><ver>:filas`. All typed fields are
/// optional: rows differ by product type. Unknown keys land in `extra`.
///
/// Scalars are read leniently (a JSON number or boolean where text is
/// expected becomes its text), and prices accept numbers or numeric text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NtRow {
    /// Hash field the row came from: `<id>` or `<id>#N`. Not part of the JSON.
    #[serde(skip)]
    pub id: String,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub cum_facturar: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub cod_auf: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub cod_mutual: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub cod_gen_siesa: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub tipo_producto: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub ffm_rips: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub umm_rips: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub upr_rips: Option<String>,
    #[serde(
        default,
        deserialize_with = "money",
        serialize_with = "ser_money",
        skip_serializing_if = "Option::is_none"
    )]
    pub precio_contrato: Option<Decimal>,
    #[serde(
        default,
        deserialize_with = "money",
        serialize_with = "ser_money",
        skip_serializing_if = "Option::is_none"
    )]
    pub precio_final: Option<Decimal>,
    #[serde(
        default,
        deserialize_with = "money",
        serialize_with = "ser_money",
        skip_serializing_if = "Option::is_none"
    )]
    pub precio_maximo: Option<Decimal>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub subplan: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub contrato: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub clasificacion: Option<String>,
    #[serde(
        default,
        deserialize_with = "text",
        skip_serializing_if = "Option::is_none"
    )]
    pub estado: Option<String>,
    /// Every other key of the JSON object.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl NtRow {
    /// Parses the JSON stored in the collection; `id` is the hash field.
    pub fn from_json(id: &str, json: &str) -> Result<NtRow, serde_json::Error> {
        let mut row: NtRow = serde_json::from_str(json)?;
        row.id = id.to_owned();
        Ok(row)
    }

    /// Text of a field by name (typed or extra), as the `idx` sets index it.
    pub fn get(&self, field: &str) -> Option<String> {
        let typed = match field {
            "cum_facturar" => &self.cum_facturar,
            "cod_auf" => &self.cod_auf,
            "cod_mutual" => &self.cod_mutual,
            "cod_gen_siesa" => &self.cod_gen_siesa,
            "tipo_producto" => &self.tipo_producto,
            "ffm_rips" => &self.ffm_rips,
            "umm_rips" => &self.umm_rips,
            "upr_rips" => &self.upr_rips,
            "subplan" => &self.subplan,
            "contrato" => &self.contrato,
            "clasificacion" => &self.clasificacion,
            "estado" => &self.estado,
            "precio_contrato" => return self.precio_contrato.map(|d| d.to_string()),
            "precio_final" => return self.precio_final.map(|d| d.to_string()),
            "precio_maximo" => return self.precio_maximo.map(|d| d.to_string()),
            other => return self.extra.get(other).and_then(scalar_text),
        };
        typed.clone()
    }

    /// Base id without the `#N` variant suffix.
    pub fn base_id(&self) -> &str {
        base_id(&self.id)
    }
}

/// `"123#2"` -> `"123"`.
pub fn base_id(field: &str) -> &str {
    field.split('#').next().unwrap_or(field)
}

fn scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn text<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(de)?;
    Ok(value.as_ref().and_then(scalar_text))
}

fn money<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Decimal>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(de)?;
    let text = match value {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::String(s)) => s.trim().to_owned(),
        Some(other) => {
            return Err(serde::de::Error::custom(format!(
                "precio inválido: {other}"
            )));
        }
    };
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<Decimal>()
        .or_else(|_| Decimal::from_scientific(&text))
        .map(Some)
        .map_err(|_| serde::de::Error::custom(format!("precio inválido: {text}")))
}

fn ser_money<S: Serializer>(value: &Option<Decimal>, ser: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(d) => ser.serialize_str(&d.to_string()),
        None => ser.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_fields_and_keeps_unknown_ones() {
        let json = r#"{"cum_facturar":"19900001-1","cod_auf":12345,"precio_contrato":1234.5,"precio_final":"99.90",
            "estado":"VIGENTE","subplan":null,"algo_nuevo":{"a":1}}"#;
        let row = NtRow::from_json("77#2", json).expect("row");
        assert_eq!(row.id, "77#2");
        assert_eq!(row.base_id(), "77");
        assert_eq!(row.cum_facturar.as_deref(), Some("19900001-1"));
        assert_eq!(row.cod_auf.as_deref(), Some("12345"), "numbers become text");
        assert_eq!(row.precio_contrato, Some("1234.5".parse().expect("dec")));
        assert_eq!(row.precio_final, Some("99.90".parse().expect("dec")));
        assert_eq!(row.subplan, None);
        assert!(row.extra.contains_key("algo_nuevo"));
        assert_eq!(row.get("estado").as_deref(), Some("VIGENTE"));
        assert_eq!(row.get("precio_final").as_deref(), Some("99.90"));
        assert_eq!(row.get("algo_nuevo"), None, "objects are not scalars");
        assert_eq!(row.get("zzz"), None);
    }

    #[test]
    fn empty_objects_and_blank_prices_are_fine() {
        let row = NtRow::from_json("1", r#"{"precio_maximo":""}"#).expect("row");
        assert_eq!(row.precio_maximo, None);
        assert!(NtRow::from_json("1", r#"{"precio_maximo":"abc"}"#).is_err());
        assert!(NtRow::from_json("1", "not json").is_err());
    }

    #[test]
    fn serializes_without_the_id_and_empty_fields() {
        let mut row =
            NtRow::from_json("5", r#"{"cod_auf":"A","precio_final":10,"x":1}"#).expect("row");
        row.extra.insert("y".into(), serde_json::json!(2));
        let text = serde_json::to_string(&row).expect("json");
        assert_eq!(text, r#"{"cod_auf":"A","precio_final":"10","x":1,"y":2}"#);
    }
}
