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

/// The default weekly comparison pair (ADR-0022): the newest completed run of
/// the latest import revision and of the immediately preceding revision of the
/// same source profile. A null member means the comparison is unavailable.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WeeklyComparisonContext {
    pub current: Option<RunSummary>,
    pub previous: Option<RunSummary>,
}

/// One changed series in the weekly comparison list.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesChangeSummary {
    pub product_id: String,
    pub location_id: String,
    pub location_code: String,
    pub series_key: String,
    pub demand_change_count: u32,
    pub forecast_change_count: u32,
    /// `both`, `current_only`, or `previous_only`.
    pub presence: String,
}

/// One changing observation cell between two import revisions.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DemandChangePoint {
    pub week_start: String,
    pub previous_quantity: Option<f64>,
    pub previous_status: Option<String>,
    pub current_quantity: Option<f64>,
    pub current_status: Option<String>,
    /// `added`, `removed`, or `changed`.
    pub change_kind: String,
}

/// One week of the baseline forecast comparison between two runs.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ForecastChangePoint {
    pub week_start: String,
    pub previous_value: Option<f64>,
    pub current_value: Option<f64>,
    pub delta: Option<f64>,
    /// `common`, `previous_only`, or `current_only`.
    pub comparability: String,
    /// True only for differing values and availability changes inside the other
    /// run's horizon; horizon edges are not changes (ADR-0022).
    pub is_change: bool,
}

/// The weekly comparison detail for one series.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesComparisonDetail {
    pub product_id: String,
    pub location_id: String,
    pub location_code: String,
    pub series_key: String,
    pub presence: String,
    pub current_outcome: Option<String>,
    pub previous_outcome: Option<String>,
    pub demand_changes: Vec<DemandChangePoint>,
    pub forecast_changes: Vec<ForecastChangePoint>,
    pub planning: Vec<PlanningPoint>,
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
