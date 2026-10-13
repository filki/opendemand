use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceInfo {
    pub path: String,
    pub schema_version: String,
    pub latest_revision_id: Option<i32>,
    pub recovery_issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ImportResult {
    pub revision_id: i32,
    pub is_duplicate: bool,
    pub series_count: u32,
    pub observation_count: u32,
    pub issue_count: u32,
    pub last_week: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesSummary {
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub h_length: u32,
    pub qualification_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HistoryPoint {
    pub week_start: String,
    pub quantity: Option<f64>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ForecastPoint {
    pub week_start: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesDetail {
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub history: Vec<HistoryPoint>,
    pub forecast: Vec<ForecastPoint>,
    pub run_id: Option<String>,
    pub outcome: Option<String>,
    pub reason: Option<String>,
    pub horizon_start_week: Option<String>,
    pub selected_model_id: Option<String>,
    pub selection_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RunSummary {
    pub id: String,
    pub revision_id: i32,
    pub status: String,
    pub created_at: String,
    pub horizon_start_week: String,
    pub engine_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ForecastResult {
    pub run_id: String,
    pub revision_id: i32,
    pub status: String,
    pub series_count: u32,
    pub forecast_count: u32,
    pub coverage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct JobInfo {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub phase: String,
    pub progress: f64,
    pub error: Option<crate::error::AppError>,
    pub import_result: Option<ImportResult>,
    pub forecast_result: Option<ForecastResult>,
}

#[derive(Debug, Clone)]
pub struct ObservationRecord {
    pub week_start: NaiveDate,
    pub quantity: Option<f64>,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct InputSeries {
    pub product_id: Uuid,
    pub location_id: Uuid,
    pub series_key: String,
    pub h_length: u32,
    pub qualification_status: String,
    pub reason: Option<String>,
    pub observations: Vec<ObservationRecord>,
}

#[derive(Debug, Clone)]
pub struct ForecastInput {
    pub revision_id: i32,
    pub cutoff: NaiveDate,
    pub timezone: String,
    pub series: Vec<InputSeries>,
}
