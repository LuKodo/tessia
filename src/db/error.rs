//! Database error type with a transient-failure classifier.

use diesel::result::{DatabaseErrorKind, Error as DieselError};

/// Any failure talking to Postgres.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// A statement or transaction failed.
    #[error("error de base de datos: {0}")]
    Query(#[from] DieselError),
    /// No pooled connection could be obtained (or the pool could not be built).
    #[error("sin conexión a la base de datos: {0}")]
    Pool(#[from] r2d2::Error),
    /// Opening a connection failed.
    #[error("no se pudo conectar a la base de datos: {0}")]
    Connect(#[from] diesel::ConnectionError),
    /// A failure that is not a Postgres error (bad data, invariant).
    #[error("{0}")]
    Other(String),
}

/// Result alias of every store function.
pub type DbResult<T> = Result<T, DbError>;

impl DbError {
    /// Builds an [`DbError::Other`].
    pub fn other(message: impl Into<String>) -> Self {
        DbError::Other(message.into())
    }

    /// True when running the same unit of work again on a fresh connection can
    /// succeed: serialization failure (40001), deadlock (40P01), a closed or
    /// lost connection, or a pool that timed out.
    pub fn is_transient(&self) -> bool {
        match self {
            DbError::Pool(_) | DbError::Connect(_) => true,
            DbError::Query(err) => query_is_transient(err),
            DbError::Other(_) => false,
        }
    }

    /// True for a unique-constraint violation (23505).
    pub fn is_unique_violation(&self) -> bool {
        matches!(
            self,
            DbError::Query(DieselError::DatabaseError(
                DatabaseErrorKind::UniqueViolation,
                _
            ))
        )
    }
}

fn query_is_transient(err: &DieselError) -> bool {
    match err {
        DieselError::DatabaseError(
            DatabaseErrorKind::SerializationFailure | DatabaseErrorKind::ClosedConnection,
            _,
        ) => true,
        // Diesel has no kind for 40P01 (deadlock) nor for connections that die
        // mid-statement, so the server/libpq message decides.
        DieselError::DatabaseError(DatabaseErrorKind::Unknown, info) => {
            message_is_transient(info.message())
        }
        DieselError::BrokenTransactionManager => false,
        _ => false,
    }
}

fn message_is_transient(message: &str) -> bool {
    const MARKERS: [&str; 8] = [
        "deadlock detected",
        "could not serialize access",
        "server closed the connection",
        "terminating connection",
        "connection reset",
        "connection refused",
        "no connection to the server",
        "SSL SYSCALL error",
    ];
    MARKERS.iter().any(|m| message.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_kind_and_message() {
        let closed = DbError::Query(DieselError::DatabaseError(
            DatabaseErrorKind::ClosedConnection,
            Box::new("x".to_owned()),
        ));
        assert!(closed.is_transient());
        let deadlock = DbError::Query(DieselError::DatabaseError(
            DatabaseErrorKind::Unknown,
            Box::new("deadlock detected".to_owned()),
        ));
        assert!(deadlock.is_transient());
        let unique = DbError::Query(DieselError::DatabaseError(
            DatabaseErrorKind::UniqueViolation,
            Box::new("dup".to_owned()),
        ));
        assert!(!unique.is_transient() && unique.is_unique_violation());
        assert!(!DbError::other("bad data").is_transient());
        assert!(!DbError::Query(DieselError::NotFound).is_transient());
    }
}
