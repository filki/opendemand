use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppError {
    pub code: String,
    pub message: String,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        eprintln!("Database operation failed: {error}");
        Self::new(
            "database_error",
            "Database operation failed. Check the workspace and retry.",
        )
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::new(
            "io_error",
            format!("File operation failed ({:?}).", error.kind()),
        )
    }
}

impl From<serde_json::Error> for AppError {
    fn from(_: serde_json::Error) -> Self {
        Self::new(
            "contract_violation",
            "Invalid JSON received from the forecasting engine.",
        )
    }
}

impl From<rust_xlsxwriter::XlsxError> for AppError {
    fn from(_: rust_xlsxwriter::XlsxError) -> Self {
        Self::new(
            "export_failed",
            "Could not write the XLSX workbook. Check the destination, free space, and worksheet limits.",
        )
    }
}
