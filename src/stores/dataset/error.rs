use crate::db::DbError;

/// Failure of a [`DatasetStore`](super::DatasetStore) call.
#[derive(Debug, thiserror::Error)]
pub enum DatasetError {
    /// The cargue has no stored dataset (Go `ErrDatasetNotFound`).
    #[error("el cargue #{0} no tiene dataset preparado")]
    NotFound(i64),
    /// No upload with this `cargues.seq` exists.
    #[error("el cargue #{0} no existe")]
    CargueNotFound(i64),
    /// An invoice changed since the version the caller read (Go
    /// `ErrDatasetVersionConflict`). Retryable: read again and redo the work.
    #[error(
        "la factura {invoice} cambió desde que se leyó (versión guardada {stored}, esperada {expected}): vuelva a ejecutar el comando"
    )]
    VersionConflict {
        invoice: String,
        stored: i64,
        expected: i64,
    },
    /// The invoice is not a stored, prepared invoice of the cargue (Go
    /// `ErrDatasetInvoiceNotFound`).
    #[error("la factura {invoice} no es una factura preparada del cargue #{cargue}")]
    InvoiceNotFound { cargue: i64, invoice: String },
    /// The data handed in is inconsistent (repeated invoice, unreadable
    /// stored JSON, ...).
    #[error("dataset inválido: {0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] DbError),
}

impl DatasetError {
    /// True for [`DatasetError::VersionConflict`]: re-read the invoices and
    /// redo the update.
    pub fn is_version_conflict(&self) -> bool {
        matches!(self, DatasetError::VersionConflict { .. })
    }

    /// Alias of [`is_version_conflict`](Self::is_version_conflict): the only
    /// error where trying the whole unit again can succeed (database
    /// transients are already retried inside the store).
    pub fn is_retryable(&self) -> bool {
        self.is_version_conflict()
    }
}

impl From<diesel::result::Error> for DatasetError {
    fn from(err: diesel::result::Error) -> Self {
        DatasetError::Db(DbError::Query(err))
    }
}

pub type DatasetResult<T> = Result<T, DatasetError>;
