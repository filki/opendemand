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

/// One row of the home screen's workspace list: a recently opened workspace
/// plus whether its database still exists on disk.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceRef {
    pub path: String,
    pub name: String,
    pub last_opened_at: String,
    pub exists: bool,
    pub is_default: bool,
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
    /// Reason recorded with the latest override revision; NULL with a non-zero
    /// revision means a legacy override written before reasons were required.
    pub override_reason: Option<String>,
    /// Free-text note recorded with the latest override revision.
    pub override_note: Option<String>,
    /// Optional reminder date (`YYYY-MM-DD`) recorded with the latest override
    /// revision; a reminder only, never automatic expiry (ADR-0027).
    pub override_review_date: Option<String>,
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
    /// Required planner reason; one of the documented reason codes (ADR-0027).
    pub reason_code: String,
    /// Optional free-text note; required for the `other` reason.
    pub note: Option<String>,
    /// Optional reminder date (`YYYY-MM-DD`); reminder only, never expiry.
    pub review_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesNote {
    pub id: i32,
    pub note: String,
    /// The run that was open when the note was written, when one was supplied.
    pub context_run_id: Option<String>,
    pub created_at: String,
}

/// Derived review state of one series in one run (ADR-0027). A stored
/// acknowledgement whose evidence fingerprint no longer matches the run's
/// current evidence reads as `unreviewed`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesReviewInfo {
    /// `reviewed` or `unreviewed`.
    pub status: String,
    /// When the current acknowledgement was recorded; None when unreviewed.
    pub reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesNoteEdit {
    pub run_id: String,
    pub product_id: String,
    pub location_id: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReviewActionEdit {
    pub run_id: String,
    pub product_id: String,
    pub location_id: String,
    /// `reviewed` or `reopened`.
    pub action: String,
}

/// Versioned, profile-scoped thresholds for the derived exception inbox
/// (ADR-0028). Both thresholds are NULL together (rule disabled) or set.
/// The persistent-bias group (horizon, window, minimum, threshold) is
/// all-set or all-null as well (ADR-0030).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InboxPolicy {
    /// Absolute material-change threshold in the series' own quantity units.
    pub change_abs: Option<f64>,
    /// Relative material-change threshold as a fraction of the previous
    /// baseline magnitude (0.25 means 25%).
    pub change_rel: Option<f64>,
    /// Horizon step the persistent-bias rule evaluates (1..12).
    pub bias_horizon: Option<u32>,
    /// How many mature target weeks the rule considers.
    pub bias_window: Option<u32>,
    /// Minimum mature target weeks before the rule can fire (at least 2).
    pub bias_min: Option<u32>,
    /// Minimum absolute pooled bias before the rule can fire (> 0).
    pub bias_abs: Option<f64>,
    /// Latest stored policy version; None when never configured.
    pub version: Option<i32>,
}

/// Raw threshold input; empty strings disable the rule. Each rule group must
/// be provided together or left empty.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InboxPolicyEdit {
    pub change_abs: Option<String>,
    pub change_rel: Option<String>,
    pub bias_horizon: Option<String>,
    pub bias_window: Option<String>,
    pub bias_min: Option<String>,
    pub bias_abs: Option<String>,
}

/// One derived exception-inbox row: a series with at least one applicable rule
/// for the selected run. Nothing here is stored; rules, fingerprints, and
/// review state are computed at read time (ADR-0028).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InboxItem {
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub location_code: String,
    pub outcome: String,
    pub reason: Option<String>,
    /// Rule codes ordered by the fixed priority; the first entry set the row order.
    pub rules: Vec<String>,
    /// Sum of absolute baseline deltas over common calendar weeks.
    pub change_abs: Option<f64>,
    /// Relative change; null when the previous baseline magnitude is zero
    /// (the percentage is unavailable, never invented).
    pub change_rel: Option<f64>,
    /// Earliest due override review date for this series, when any is due.
    pub due_review_date: Option<String>,
    /// Pooled persistent bias (positive means overforecasting) when the rule
    /// fired; None otherwise.
    pub bias_value: Option<f64>,
    /// Mature target weeks the bias evidence covered.
    pub bias_weeks: Option<u32>,
    /// Horizon step the bias evidence was evaluated at.
    pub bias_horizon: Option<u32>,
    /// `reviewed` or `unreviewed` for the selected run.
    pub review_status: String,
    pub reviewed_at: Option<String>,
    /// Prior notes attached to this series (evidence link).
    pub note_count: u32,
    pub last_note_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ExceptionInbox {
    pub run_id: String,
    pub policy: InboxPolicy,
    /// Newest completed run of the immediately preceding revision; None when
    /// no comparison evidence exists (the material-change rule is unavailable).
    pub predecessor_run_id: Option<String>,
    /// True when material-change rules were evaluated against a predecessor run.
    pub material_change_available: bool,
    /// Rows matching the search and filter across the whole run.
    pub total: u32,
    pub rows: Vec<InboxItem>,
}

/// One immutable official publication of a completed run (ADR-0029). Published
/// effective values are derived from the pinned override revisions plus the
/// immutable baselines; nothing is copied.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PlanPublication {
    pub id: i32,
    pub run_id: String,
    pub revision_id: i32,
    pub cutoff_week: String,
    pub horizon_start_week: String,
    /// Highest import revision id of the profile visible at publication time.
    pub revision_high_water: i32,
    pub published_at: String,
    /// Overridden cells pinned by the publication.
    pub override_cells: u32,
    /// Distinct series with at least one pinned override cell.
    pub override_series: u32,
    pub series_count: u32,
    pub covered_series: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PublicationEdit {
    pub run_id: String,
}

/// The published plan for one series: the same point shape as the live planner,
/// but with override values pinned at publication time (ADR-0029).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PublicationSeriesDetail {
    pub publication_id: i32,
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub location_code: String,
    pub outcome: String,
    pub reason: Option<String>,
    pub planning: Vec<PlanningPoint>,
}

/// One append-only confirmation that an actuals revision is complete through a
/// finished week (ADR-0030). The latest confirmation of the profile is current.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ActualsConfirmation {
    pub id: i32,
    pub revision_id: i32,
    pub confirmed_through: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ActualsConfirmationEdit {
    pub revision_id: i32,
    /// ISO Monday, no later than the last finished week.
    pub confirmed_through: String,
}

/// Why one target point could not be scored (ADR-0030). Counts only; nothing
/// is stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct FvaExclusions {
    /// Target week beyond the confirmed completed range.
    pub future: u32,
    /// The publication happened at or after the target week started.
    pub late_published: u32,
    /// The target actual was visible to the planner at or before the
    /// publication's revision high-water mark.
    pub hindsight: u32,
    /// No observation row for the target week in the confirmed revision.
    pub absent: u32,
    /// The target observation is missing or partial.
    pub missing_or_partial: u32,
    /// The target observation is marked as a stockout.
    pub stockout: u32,
    /// The published plan has no baseline or effective value for the week.
    pub no_forecast: u32,
}

/// Same-unit measures pooled over eligible points of one series (ADR-0030).
/// WAPE and bias are unavailable when the pooled actual magnitude is zero;
/// MAE stays available in quantity units.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct FvaSeriesMeasures {
    pub points: u32,
    pub baseline_wape: Option<f64>,
    pub effective_wape: Option<f64>,
    /// Baseline WAPE minus effective WAPE in percentage points; positive means
    /// the adjustment improved the plan.
    pub wape_improvement_pp: Option<f64>,
    pub baseline_mae: Option<f64>,
    pub effective_mae: Option<f64>,
    /// Baseline MAE minus effective MAE; positive means improvement.
    pub mae_improvement: Option<f64>,
    pub baseline_bias: Option<f64>,
    pub effective_bias: Option<f64>,
}

/// Derived retrospective aggregates for one publication at one horizon step.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PublicationFva {
    pub publication_id: i32,
    pub run_id: String,
    pub revision_id: i32,
    pub cutoff_week: String,
    pub horizon: u32,
    pub scored: u32,
    pub adjusted: u32,
    /// Scored points whose run input had imputed weeks (disclosure only).
    pub imputed_input: u32,
    /// Series whose absolute error got smaller, larger, or stayed equal.
    pub improved: u32,
    pub harmful: u32,
    pub unchanged: u32,
    pub excluded: FvaExclusions,
    /// True when fewer than five points scored; no significance is implied.
    pub low_sample: bool,
}

/// Per-series retrospective view pooled across publications at one horizon.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SeriesFvaRow {
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub location_code: String,
    pub scored: u32,
    pub adjusted: u32,
    pub measures: FvaSeriesMeasures,
    /// True when any scored point's run input had imputed weeks.
    pub imputed_input: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct FvaReport {
    pub horizon: u32,
    /// Latest actuals confirmation of the profile; None until confirmed.
    pub confirmation: Option<ActualsConfirmation>,
    pub publications: Vec<PublicationFva>,
    /// Series rows matching the search across every publication.
    pub total: u32,
    pub rows: Vec<SeriesFvaRow>,
}

/// One target point of one publication (scored or excluded), with all inputs
/// visible: actual from the confirmed revision, pinned published values, and
/// forecast-minus-actual errors (positive means overforecast).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct FvaPoint {
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub location_code: String,
    pub week_start: String,
    pub actual: Option<f64>,
    pub baseline: Option<f64>,
    pub effective: Option<f64>,
    pub baseline_error: Option<f64>,
    pub effective_error: Option<f64>,
    /// Effective minus baseline; None when either value is missing.
    pub adjustment: Option<f64>,
    pub imputed_input: bool,
    /// Exclusion code when the point was not scored; None when scored.
    pub excluded: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PublicationFvaPoints {
    pub publication_id: i32,
    pub horizon: u32,
    pub total: u32,
    pub rows: Vec<FvaPoint>,
}

/// Inputs for one inventory/1 import (ADR-0031): either three CSV files or one
/// XLSX workbook with the exact sheets `Stock`, `Receipts`, and `Policy`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InventoryImportEdit {
    pub stock_path: Option<String>,
    pub receipts_path: Option<String>,
    pub policy_path: Option<String>,
    pub workbook_path: Option<String>,
    /// Required when the batch has no receipts at all.
    pub receipts_empty_confirmed: bool,
    /// Confirms stock and receipts use the declared unit and the forecast does too.
    pub unit_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InventoryFinding {
    /// `blocking` or `warning`.
    pub severity: String,
    pub code: String,
    pub detail: String,
    pub row_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InventoryPreview {
    pub as_of_week: Option<String>,
    pub sku_count: u32,
    pub stock_count: u32,
    pub receipt_count: u32,
    pub policy_count: u32,
    pub overdue_receipts: u32,
    pub beyond_horizon_receipts: u32,
    pub findings: Vec<InventoryFinding>,
    pub can_import: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InventorySnapshot {
    pub id: i32,
    pub as_of_week: String,
    pub imported_at: String,
    pub sku_count: u32,
    pub receipt_count: u32,
    pub overdue_receipts: u32,
    /// True when identical content already existed; nothing was written.
    pub is_duplicate: bool,
}

/// Sensitivities and selection for one derived replenishment plan (ADR-0031).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReplenishmentEdit {
    pub publication_id: i32,
    pub snapshot_id: i32,
    /// Raw decimal multipliers; both optional, `0 <= low <= 1 <= high`.
    pub low_multiplier: Option<String>,
    pub high_multiplier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReplenishmentWeek {
    /// 1-based week of the protection/projection window: `anchor + (index-1)` weeks.
    pub index: u32,
    pub week_start: String,
    /// Published effective demand for the week; null blocks when inside P.
    pub demand: Option<f64>,
    /// Open receipts arriving at the beginning of this week (already in stock
    /// receipts never appear here).
    pub receipt: f64,
    /// Projected closing balance without the suggested order; null when demand
    /// is unknown.
    pub closing: Option<f64>,
    /// True when the suggested order arrives at the beginning of this week.
    pub order_arrival: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReplenishmentRow {
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub location_code: String,
    pub unit: String,
    pub lead_weeks: u32,
    pub review_weeks: u32,
    pub protection_weeks: u32,
    pub safety_stock: f64,
    pub moq: f64,
    pub pack_size: f64,
    pub opening_stock: f64,
    pub inbound_in_window: f64,
    /// Aggregate demand over the protection window; null when blocked.
    pub demand_window: Option<f64>,
    pub raw_need: Option<f64>,
    pub order_base: Option<f64>,
    pub order_low: Option<f64>,
    pub order_high: Option<f64>,
    /// Blocking reasons (no order is suggested while any exists).
    pub blocked: Vec<InventoryFinding>,
    /// Non-blocking disclosures (overdue receipts, imputed input, receipts
    /// beyond the horizon, early shortages).
    pub warnings: Vec<InventoryFinding>,
    pub weeks: Vec<ReplenishmentWeek>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReplenishmentPlan {
    pub publication_id: i32,
    pub snapshot_id: i32,
    pub anchor_week: String,
    pub cutoff_week: String,
    pub horizon_start_week: String,
    pub low_multiplier: Option<f64>,
    pub high_multiplier: Option<f64>,
    pub total: u32,
    pub rows: Vec<ReplenishmentRow>,
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
    /// The imputation method the selected run planned with (ADR-0023), or NULL
    /// when the run used no imputation.
    pub imputation_method: Option<String>,
    /// Missing weeks the run filled for this series; zero without imputation.
    pub imputed_weeks: Option<u32>,
    /// Profile-scoped planner notes, newest first; independent of the run.
    pub notes: Vec<SeriesNote>,
    /// Derived review state for the selected run; None without a run.
    pub review: Option<SeriesReviewInfo>,
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
pub struct RunException {
    pub product_id: String,
    pub location_id: String,
    pub series_key: String,
    pub location_code: String,
    pub outcome: String,
    pub reason: Option<String>,
}

/// Persisted model coverage, not accuracy or manual-plan completeness.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RunOverview {
    pub series_count: u32,
    pub covered_series: u32,
    pub coverage: f64,
    pub outcomes: std::collections::BTreeMap<String, u32>,
    pub exception_count: u32,
    pub exceptions: Vec<RunException>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyChangeFilter {
    All,
    Demand,
    Forecast,
    Presence,
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
    /// Zero-based positions of weeks filled by model-input imputation
    /// (ADR-0023); empty when the run uses no imputation. The planner excludes
    /// backtest origins whose target window contains one of these weeks.
    pub imputed_indices: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct ForecastInput {
    pub revision_id: i32,
    pub cutoff: NaiveDate,
    pub timezone: String,
    pub series: Vec<InputSeries>,
}
