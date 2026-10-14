use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceInfo {
    pub path: String,
    pub schema_version: String,
    pub latest_revision_id: Option<i32>,
    /// Workspace locale from `workspace_settings`; the guided import copy
    /// follows it (en, pl, es).
    pub locale: String,
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
    pub location_code: String,
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
pub struct PlanningPoint {
    pub week_start: String,
    pub horizon: u32,
    pub baseline: Option<f64>,
    pub override_value: Option<f64>,
    pub effective: Option<f64>,
    pub value_source: String,
    pub override_revision: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct OverrideEdit {
    pub run_id: String,
    pub product_id: String,
    pub location_id: String,
    pub week_start: String,
    // Preserve the input text across IPC so NaN, infinity, and blanks cannot
    // become an accidental NULL clear through JSON number serialization.
    pub value: Option<String>,
    pub expected_revision: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct OverrideRevision {
    pub revision: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ExportResult {
    pub export_id: String,
    pub path: String,
    pub row_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesDetail {
    pub product_id: String,
    pub location_id: String,
    pub location_code: String,
    pub series_key: String,
    pub history: Vec<HistoryPoint>,
    pub forecast: Vec<ForecastPoint>,
    pub planning: Vec<PlanningPoint>,
    pub can_override: bool,
    pub run_id: Option<String>,
    pub outcome: Option<String>,
    pub reason: Option<String>,
    pub horizon_start_week: Option<String>,
    pub selected_model_id: Option<String>,
    pub selection_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelOriginRow {
    pub origin_index: u32,
    pub origin_week_start: String,
    pub train_size: u32,
    pub status: String,
    pub wape: Option<f64>,
    pub mae: Option<f64>,
    pub metric_kind: Option<String>,
    pub failure_code: Option<String>,
}

/// One model's comparison row for a series in a run. `selection_score` is the
/// comparable pooled score persisted by Rust (see ADR-0017); `wape`, `mae`, and
/// `bias` are pooled diagnostics over the model's own completed origins,
/// computed from the retained held-out backtest forecasts. Those windows can
/// differ between models, so only `selection_score` is comparable across
/// models. Positive bias means overforecasting.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesModelComparison {
    pub model_id: String,
    pub eligibility_status: String,
    pub origins_planned: u32,
    pub origins_completed: u32,
    pub selection_score: Option<f64>,
    pub selection_rank: Option<u32>,
    pub is_champion: bool,
    pub production_status: String,
    pub failure_code: Option<String>,
    pub failure_detail: Option<String>,
    pub comparable_metric: Option<String>,
    pub wape: Option<f64>,
    pub mae: Option<f64>,
    pub bias: Option<f64>,
    pub selected: bool,
    pub origins: Vec<ModelOriginRow>,
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
