//! `derechos_afiliado`: cache of validated affiliate rights.
//!
//! Only complete records are stored (the validator service guarantees it).
//! A cache hit must be at least as recent as `not_before`
//! (`now - cache TTL`), mirroring Go `RightsCache`.

mod schema;

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::upsert::excluded;

use crate::db::{DbPool, DbResult};
use schema::derechos_afiliado as t;

/// Rights of an affiliate as the validator returns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rights {
    /// `01` contributivo, `04` subsidiado.
    pub user_type: String,
    /// `U` urbana, `R` rural.
    pub zone: String,
    /// Municipality DANE code.
    pub dane: String,
    /// `M`, `F`, `I`.
    pub sex: String,
    /// As returned (`yyyy-MM-dd`).
    pub birth_date: String,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = t)]
struct Row {
    tipo_usuario: String,
    zona: String,
    municipio_dane: String,
    sexo: String,
    fecha_nacimiento: String,
}

#[derive(Clone)]
pub struct DerechosCache {
    pool: DbPool,
}

impl DerechosCache {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Rights of `(doc_type, doc_number)` validated at or after `not_before`.
    pub fn get(
        &self,
        doc_type: &str,
        doc_number: &str,
        not_before: DateTime<Utc>,
    ) -> DbResult<Option<Rights>> {
        self.pool.run("derechos cache get", |conn| {
            let row: Option<Row> = t::table
                .filter(
                    t::tipo_documento
                        .eq(doc_type)
                        .and(t::numero_documento.eq(doc_number))
                        .and(t::validado_at.ge(not_before)),
                )
                .select(Row::as_select())
                .first(conn)
                .optional()?;
            Ok(row.map(|r| Rights {
                user_type: r.tipo_usuario,
                zone: r.zona,
                dane: r.municipio_dane,
                sex: r.sexo,
                birth_date: r.fecha_nacimiento,
            }))
        })
    }

    /// Stores (or refreshes) the rights validated at `at`.
    pub fn put(
        &self,
        doc_type: &str,
        doc_number: &str,
        rights: &Rights,
        at: DateTime<Utc>,
    ) -> DbResult<()> {
        self.pool.run("derechos cache put", |conn| {
            diesel::insert_into(t::table)
                .values((
                    t::tipo_documento.eq(doc_type),
                    t::numero_documento.eq(doc_number),
                    t::tipo_usuario.eq(&rights.user_type),
                    t::zona.eq(&rights.zone),
                    t::municipio_dane.eq(&rights.dane),
                    t::sexo.eq(&rights.sex),
                    t::fecha_nacimiento.eq(&rights.birth_date),
                    t::validado_at.eq(at),
                ))
                .on_conflict((t::tipo_documento, t::numero_documento))
                .do_update()
                .set((
                    t::tipo_usuario.eq(excluded(t::tipo_usuario)),
                    t::zona.eq(excluded(t::zona)),
                    t::municipio_dane.eq(excluded(t::municipio_dane)),
                    t::sexo.eq(excluded(t::sexo)),
                    t::fecha_nacimiento.eq(excluded(t::fecha_nacimiento)),
                    t::validado_at.eq(excluded(t::validado_at)),
                ))
                .execute(conn)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::db::test_support::TestDb;

    fn rights(zone: &str) -> Rights {
        Rights {
            user_type: "01".into(),
            zone: zone.into(),
            dane: "13001".into(),
            sex: "F".into(),
            birth_date: "1990-01-31".into(),
        }
    }

    #[test]
    fn hits_respect_the_freshness_bound() {
        let db = TestDb::new();
        let cache = DerechosCache::new(db.pool());
        let validated = Utc::now() - Duration::days(2);
        cache
            .put("CC", "123", &rights("U"), validated)
            .expect("put");
        let fresh_enough = cache
            .get("CC", "123", validated - Duration::hours(1))
            .expect("get");
        assert_eq!(fresh_enough, Some(rights("U")));
        assert_eq!(
            cache
                .get("CC", "123", Utc::now() - Duration::days(1))
                .expect("too old"),
            None
        );
        assert_eq!(
            cache
                .get("TI", "123", validated - Duration::hours(1))
                .expect("other doc type"),
            None
        );
    }

    #[test]
    fn put_refreshes_an_existing_record() {
        let db = TestDb::new();
        let cache = DerechosCache::new(db.pool());
        let old = Utc::now() - Duration::days(30);
        cache.put("CC", "1", &rights("U"), old).expect("old");
        cache.put("CC", "1", &rights("R"), Utc::now()).expect("new");
        let got = cache
            .get("CC", "1", Utc::now() - Duration::days(1))
            .expect("get")
            .expect("hit");
        assert_eq!(got.zone, "R");
    }
}
