//! Forecasting orchestration: planning, sidecar execution, and persistence.
//!
//! ADR-0006 fixes the model set, per-model training floors, the 12-week
//! horizon, and at most four rolling origins per model. ADR-0012 keeps
//! evaluation per series x model x origin; ADR-0017 defines the comparable
//! pooled selection scores this module persists. Rust plans and selects;
//! Python only fits, predicts, and computes per-origin metrics.

use crate::{
    engine::{self, EngineOutput, Progress, Task},
    error::{AppError, AppResult},
    imputation::{self, ImputationMethod},
    selection::{self, ModelEvidence},
    types::{ForecastInput, ForecastResult},
    workspace::Workspace,
};
use chrono::{Duration, Utc};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use uuid::Uuid;

/// Model ids with their training floors (ADR-0006). SeasonalNaive and AutoETS
/// use two full 52-week cycles; verified against statsforecast 2.1.1, where
/// SeasonalNaive needs at least 52 observations for finite forecasts and
/// AutoETS fits short, zero, negative, and intermittent histories.
pub const MODELS: [(&str, i32); 3] = [
    ("naive", 12),
    ("seasonal_naive_52", 104),
    ("auto_ets_52", 104),
];
const HORIZON_WEEKS: i32 = 12;
const MAX_ORIGINS: i32 = 4;

fn cancelled(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(AppError::new(
            "cancelled",
            "Forecast was cancelled before persistence.",
        ))
    } else {
        Ok(())
    }
}

fn plan_tasks(input: &ForecastInput) -> AppResult<Vec<Task>> {
    let mut tasks = Vec::new();
    for series in &input.series {
        if series.qualification_status != "eligible" {
            continue;
        }
        let h = series.h_length as i32;
        let start = input
            .cutoff
            .checked_sub_signed(Duration::weeks((h - 1) as i64))
            .ok_or_else(|| {
                AppError::new(
                    "invalid_input",
                    "Series history exceeds the supported calendar.",
                )
            })?;
        for (model_id, floor) in MODELS {
            if h < floor {
                continue;
            }
            let mut add_task = |kind: &str, index: Option<i32>, train_size: i32| {
                let train_end = start + Duration::weeks((train_size - 1) as i64);
                tasks.push(Task {
                    id: format!(
                        "{}:{}:{}:{kind}:{}",
                        series.product_id,
                        series.location_id,
                        model_id,
                        index.unwrap_or(0)
                    ),
                    product_id: series.product_id,
                    location_id: series.location_id,
                    model_id: model_id.into(),
                    kind: kind.into(),
                    origin_index: index,
                    train_start: start,
                    train_end,
                    forecast_start: train_end + Duration::weeks(1),
                    train_size,
                });
            };
            add_task("production", None, h);
            let origins = ((h - floor) / HORIZON_WEEKS).clamp(0, MAX_ORIGINS);
            for index in 1..=origins {
                let train_size = h - HORIZON_WEEKS * (origins - index + 1);
                if target_window_imputed(&series.imputed_indices, train_size) {
                    continue;
                }
                add_task("backtest", Some(index), train_size);
            }
        }
    }
    Ok(tasks)
}

/// True when an origin's 12-week target window (zero-based positions
/// `train_size..train_size + HORIZON_WEEKS` of the effective series) contains
/// an imputed week. Such origins are neither planned nor scored, so champion
/// selection never grades models against invented values (ADR-0023).
fn target_window_imputed(imputed: &[u32], train_size: i32) -> bool {
    imputed.iter().any(|&position| {
        let position = position as i32;
        (train_size..train_size + HORIZON_WEEKS).contains(&position)
    })
}

pub async fn run_forecast(
    workspace: &Workspace,
    revision_id: i32,
    imputation: Option<ImputationMethod>,
    job_id: &str,
    cancel: Arc<AtomicBool>,
    progress: Progress,
    engine_binary: Option<PathBuf>,
) -> AppResult<ForecastResult> {
    cancelled(&cancel)?;
    progress("preparing", 0.0);
    let started_at = Utc::now().to_rfc3339();
    let mut input = workspace.forecast_input(revision_id).await?;
    if let Some(method) = imputation {
        for series in &mut input.series {
            imputation::impute_series(series, method);
        }
    }
    let tasks = plan_tasks(&input)?;
    let jobs_directory = workspace.path.join("jobs");
    std::fs::create_dir_all(&jobs_directory)?;
    let temporary = tempfile::Builder::new()
        .prefix("forecast-")
        .tempdir_in(jobs_directory)?;
    let output = engine::execute(
        temporary.path(),
        &input,
        &tasks,
        job_id,
        &cancel,
        &progress,
        engine_binary.as_deref(),
    )
    .await?;
    cancelled(&cancel)?;
    progress("persisting", 0.9);
    persist(
        workspace,
        &input,
        &tasks,
        &output,
        &started_at,
        job_id,
        imputation,
        &cancel,
    )
    .await
}

/// One model's evidence for one series: planned origins, completed origins
/// with their held-out sums, and its production task outcome.
struct SeriesModel<'a> {
    model_id: &'static str,
    production_eligible: bool,
    production: Option<&'a Task>,
    backtests: Vec<&'a Task>,
    evidence: ModelEvidence,
}

fn series_models<'a>(
    series: &crate::types::InputSeries,
    scoped: &[&'a Task],
    output: &EngineOutput,
) -> Vec<SeriesModel<'a>> {
    let h = series.h_length as i32;
    MODELS
        .into_iter()
        .map(|(model_id, floor)| {
            let model_tasks = scoped
                .iter()
                .filter(|task| task.model_id == model_id)
                .copied()
                .collect::<Vec<_>>();
            let production = model_tasks
                .iter()
                .find(|task| task.kind == "production")
                .copied();
            let mut backtests = model_tasks
                .iter()
                .filter(|task| task.kind == "backtest")
                .copied()
                .collect::<Vec<_>>();
            backtests.sort_by_key(|task| task.origin_index);
            let completed = backtests
                .iter()
                .filter(|task| output.statuses[&task.id].status == "completed")
                .filter_map(|task| {
                    output
                        .sums
                        .get(&task.id)
                        .map(|sums| (task.train_end, *sums))
                })
                .collect::<Vec<_>>();
            let production_completed =
                production.is_some_and(|task| output.statuses[&task.id].status == "completed");
            let production_eligible = series.qualification_status == "eligible" && h >= floor;
            SeriesModel {
                model_id,
                production_eligible,
                production,
                backtests,
                evidence: ModelEvidence {
                    model_id: model_id.into(),
                    production_eligible,
                    production_completed,
                    completed,
                },
            }
        })
        .collect()
}

fn eligibility_status(model: &SeriesModel<'_>) -> &'static str {
    if !model.production_eligible {
        "not_eligible"
    } else if model.evidence.completed.len() >= 2 {
        "champion_eligible"
    } else {
        "production_only"
    }
}

fn production_status<'a>(model: &SeriesModel<'_>, output: &'a EngineOutput) -> &'a str {
    match model.production {
        None => "not_run",
        Some(task) => &output.statuses[&task.id].status,
    }
}

fn outcome_for(
    series: &crate::types::InputSeries,
    models: &[SeriesModel<'_>],
    champion: Option<&String>,
    used: Option<&String>,
) -> (&'static str, Option<String>) {
    if series.qualification_status == "invalid_data" {
        return ("invalid_data", series.reason.clone());
    }
    if !models.iter().any(|model| model.production_eligible) {
        return (
            "insufficient_history",
            Some(format!(
                "Naive requires 12 consecutive known weeks; this series has {}.",
                series.h_length
            )),
        );
    }
    let Some(used) = used else {
        return (
            "model_failure",
            Some("No fallback candidate produced a forecast.".into()),
        );
    };
    match champion {
        Some(champion) if champion == used => ("ok", None),
        Some(champion) => (
            "ok",
            Some(format!(
                "Champion {champion} production failed; using {used}."
            )),
        ),
        // Without a champion the fallback chain can only have used Naive.
        None => (
            "fallback_naive",
            Some(
                if models
                    .iter()
                    .any(|model| model.evidence.completed.len() >= 2)
                {
                    "No common backtest origins across models; using the Naive baseline."
                } else {
                    "Fewer than two completed backtest origins; using the Naive baseline."
                }
                .into(),
            ),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
async fn persist(
    workspace: &Workspace,
    input: &ForecastInput,
    tasks: &[Task],
    output: &EngineOutput,
    started_at: &str,
    job_id: &str,
    imputation: Option<ImputationMethod>,
    cancel: &AtomicBool,
) -> AppResult<ForecastResult> {
    let _writer = workspace.writer.lock().await;
    cancelled(cancel)?;
    let mut transaction = workspace.pool.begin().await?;
    let run_id = Uuid::now_v7();
    let finished_at = Utc::now().to_rfc3339();
    let models = serde_json::json!(MODELS.map(|(model_id, _)| model_id));
    let floors = serde_json::Value::Object(
        MODELS
            .iter()
            .map(|(model_id, floor)| ((*model_id).to_owned(), serde_json::json!(floor)))
            .collect(),
    );
    let config = serde_json::json!({
        "models": models,
        "window_weeks": HORIZON_WEEKS,
        "max_origins": MAX_ORIGINS,
        "floors": floors,
        "selection": "comparable_pooled_wape",
        "contract": "opendemand.arrow/1",
        "imputation": imputation.map(ImputationMethod::id),
    })
    .to_string();
    sqlx::query("INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version, imputation_method) VALUES (?, ?, ?, ?, 'completed', 12, ?, ?, ?, ?, ?, ?)")
        .bind(run_id.as_bytes().as_slice()).bind(input.revision_id).bind(started_at).bind(&finished_at)
        .bind((input.cutoff + Duration::weeks(1)).to_string()).bind(&input.timezone).bind(config)
        .bind(env!("CARGO_PKG_VERSION")).bind(&output.engine_version)
        .bind(imputation.map(ImputationMethod::id)).execute(&mut *transaction).await?;

    for (model_id, _) in MODELS {
        let model_tasks = tasks
            .iter()
            .filter(|task| task.model_id == model_id)
            .collect::<Vec<_>>();
        let completed = model_tasks
            .iter()
            .filter(|task| output.statuses[&task.id].status == "completed")
            .count() as i32;
        let planned = model_tasks.len() as i32;
        let failed = planned - completed;
        let status = if planned == 0 {
            "not_run"
        } else if failed == 0 {
            "completed"
        } else if completed == 0 {
            "failed"
        } else {
            "partial"
        };
        sqlx::query("INSERT INTO run_model (run_id, model_id, status, tasks_planned, tasks_completed, tasks_failed) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(run_id.as_bytes().as_slice()).bind(model_id).bind(status).bind(planned).bind(completed).bind(failed)
            .execute(&mut *transaction).await?;
    }

    let mut series_tasks: HashMap<(Uuid, Uuid), Vec<&Task>> = HashMap::new();
    for task in tasks {
        series_tasks
            .entry((task.product_id, task.location_id))
            .or_default()
            .push(task);
    }
    let mut forecast_count = 0;
    let mut covered_series = 0;
    for series in &input.series {
        let scoped = series_tasks
            .get(&(series.product_id, series.location_id))
            .cloned()
            .unwrap_or_default();
        let models = series_models(series, &scoped, output);
        let evidences = models
            .iter()
            .map(|model| model.evidence.clone())
            .collect::<Vec<_>>();
        let selected = selection::select(&evidences);
        for model in &models {
            let ranked = selected
                .ranked
                .iter()
                .position(|score| score.model_id == model.model_id);
            let rank = ranked.map(|position| position as i32 + 1);
            let score = ranked.map(|position| selected.ranked[position].score);
            let status = production_status(model, output);
            let failure = model
                .production
                .map(|task| &output.statuses[&task.id])
                .filter(|status| status.status == "failed");
            sqlx::query("INSERT INTO series_model_evaluation (run_id, product_id, location_id, model_id, eligibility_status, origins_planned, origins_completed, selection_score, selection_rank, is_champion, production_status, failure_code, failure_detail) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
                .bind(model.model_id).bind(eligibility_status(model))
                .bind(model.backtests.len() as i32).bind(model.evidence.completed.len() as i32)
                .bind(score).bind(rank)
                .bind(i32::from(selected.champion.as_deref() == Some(model.model_id)))
                .bind(status)
                .bind(failure.and_then(|status| status.failure_code.as_deref()))
                .bind(failure.and_then(|status| status.failure_detail.as_deref()))
                .execute(&mut *transaction).await?;

            for task in &model.backtests {
                let status = &output.statuses[&task.id];
                let metric = output.origins.get(&task.id);
                sqlx::query("INSERT INTO series_model_origin (run_id, product_id, location_id, model_id, origin_index, origin_week_start, train_size, status, wape, mae, metric_kind, metric_value, failure_code, failure_detail) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                    .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
                    .bind(model.model_id).bind(task.origin_index).bind(task.train_end.to_string()).bind(task.train_size).bind(&status.status)
                    .bind(metric.and_then(|metric| metric.wape)).bind(metric.map(|metric| metric.mae)).bind(metric.map(|metric| metric.metric_kind.as_str())).bind(metric.map(|metric| metric.metric_value))
                    .bind(&status.failure_code).bind(&status.failure_detail).execute(&mut *transaction).await?;
                if let Some(points) = output.points.get(&task.id) {
                    for point in points {
                        sqlx::query("INSERT INTO backtest_forecast (run_id, product_id, location_id, model_id, origin_index, week_start, value) VALUES (?, ?, ?, ?, ?, ?, ?)")
                            .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
                            .bind(model.model_id).bind(task.origin_index).bind(&point.week_start).bind(point.value).execute(&mut *transaction).await?;
                    }
                }
            }
        }
        let (outcome, reason) = outcome_for(
            series,
            &models,
            selected.champion.as_ref(),
            selected.used.as_ref(),
        );
        sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status, selected_model_id, reason, imputed_weeks) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
            .bind(outcome).bind(selected.used.as_deref()).bind(reason)
            .bind(series.imputed_indices.len() as i64).execute(&mut *transaction).await?;
        if let Some(used) = selected.used.as_deref() {
            covered_series += 1;
            let production = models
                .iter()
                .find(|model| model.model_id == used)
                .and_then(|model| model.production)
                .expect("the used model produced a completed production task");
            for point in &output.points[&production.id] {
                sqlx::query("INSERT INTO forecast (run_id, product_id, location_id, week_start, value, selected_model_id) VALUES (?, ?, ?, ?, ?, ?)")
                    .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
                    .bind(&point.week_start).bind(point.value).bind(used).execute(&mut *transaction).await?;
                forecast_count += 1;
            }
        }
    }
    if let Ok(id) = job_id.parse::<i64>() {
        sqlx::query(
            "UPDATE job SET status='completed', finished_at=? WHERE id=? AND kind='forecast'",
        )
        .bind(&finished_at)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    }
    cancelled(cancel)?;
    transaction.commit().await?;
    Ok(ForecastResult {
        run_id: run_id.to_string(),
        revision_id: input.revision_id,
        status: "completed".into(),
        series_count: input.series.len() as u32,
        forecast_count,
        coverage: covered_series as f64 / input.series.len().max(1) as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InputSeries;
    use chrono::NaiveDate;

    fn series(h: u32) -> InputSeries {
        InputSeries {
            product_id: Uuid::now_v7(),
            location_id: Uuid::now_v7(),
            series_key: h.to_string(),
            h_length: h,
            qualification_status: if h >= 12 {
                "eligible"
            } else {
                "insufficient_history"
            }
            .into(),
            reason: None,
            observations: vec![],
            imputed_indices: Vec::new(),
        }
    }

    fn input(series: Vec<InputSeries>) -> ForecastInput {
        ForecastInput {
            revision_id: 1,
            cutoff: NaiveDate::from_ymd_opt(2026, 2, 23).unwrap(),
            timezone: "UTC".into(),
            series,
        }
    }

    fn tasks_for(h: u32, model_id: &str) -> Vec<Task> {
        plan_tasks(&input(vec![series(h)]))
            .unwrap()
            .into_iter()
            .filter(|task| task.model_id == model_id)
            .collect()
    }

    fn origin_sizes(h: u32, model_id: &str) -> Vec<i32> {
        tasks_for(h, model_id)
            .iter()
            .filter(|task| task.kind == "backtest")
            .map(|task| task.train_size)
            .collect()
    }

    #[test]
    fn naive_plan_uses_series_specific_rolling_origins_and_production_floor() {
        let tasks = plan_tasks(&input([60, 20, 6].into_iter().map(series).collect())).unwrap();
        // 60: production + 4 origins; 20: production only; 6: nothing.
        assert_eq!(tasks.len(), 6);
        let origins = tasks
            .iter()
            .filter(|task| task.kind == "backtest")
            .collect::<Vec<_>>();
        assert_eq!(
            origins
                .iter()
                .map(|task| task.train_size)
                .collect::<Vec<_>>(),
            vec![12, 24, 36, 48]
        );
        assert_eq!(
            origins
                .iter()
                .map(|task| task.train_end.to_string())
                .collect::<Vec<_>>(),
            vec!["2025-03-24", "2025-06-16", "2025-09-08", "2025-12-01"]
        );
        assert!(tasks
            .iter()
            .filter(|task| task.kind == "production")
            .all(|task| task.forecast_start.to_string() == "2026-03-02"));
    }

    #[test]
    fn every_series_model_task_id_is_globally_unique() {
        let tasks = plan_tasks(&input(vec![series(152), series(152)])).unwrap();
        let mut ids = tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total);
        assert!(tasks
            .iter()
            .any(|task| task.id.contains("seasonal_naive_52")));
        assert!(tasks.iter().any(|task| task.id.contains("auto_ets_52")));
    }

    #[test]
    fn model_thresholds_match_the_approved_table() {
        // Production floors: Naive 12, SeasonalNaive and AutoETS 104.
        assert_eq!(origin_sizes(104, "naive"), vec![56, 68, 80, 92]);
        assert!(origin_sizes(103, "seasonal_naive_52").is_empty());
        assert_eq!(origin_sizes(104, "seasonal_naive_52"), Vec::<i32>::new());
        // 116: first SeasonalNaive/AutoETS origin (train 104).
        assert_eq!(origin_sizes(116, "seasonal_naive_52"), vec![104]);
        assert_eq!(origin_sizes(116, "auto_ets_52"), vec![104]);
        // 128: two origins, so champion eligibility becomes possible.
        assert_eq!(origin_sizes(128, "seasonal_naive_52"), vec![104, 116]);
        assert_eq!(origin_sizes(128, "auto_ets_52"), vec![104, 116]);
        // 152: four origins (the maximum).
        assert_eq!(
            origin_sizes(152, "seasonal_naive_52"),
            vec![104, 116, 128, 140]
        );
        assert_eq!(origin_sizes(152, "auto_ets_52"), vec![104, 116, 128, 140]);
        // Naive thresholds stay unchanged from Milestone 1.
        assert_eq!(origin_sizes(36, "naive"), vec![12, 24]);
        assert_eq!(origin_sizes(24, "naive"), vec![12]);
        assert!(origin_sizes(23, "naive").is_empty());
    }

    #[test]
    fn origins_with_imputed_targets_are_not_planned() {
        // Position 130 (zero-based) is imputed: backtest origins whose 12-week
        // target window covers it are skipped for every model (ADR-0023).
        let mut affected = series(152);
        affected.imputed_indices = vec![130];
        let tasks = plan_tasks(&input(vec![affected])).unwrap();
        for model in ["naive", "seasonal_naive_52", "auto_ets_52"] {
            let mut sizes = tasks
                .iter()
                .filter(|task| task.kind == "backtest" && task.model_id == model)
                .map(|task| task.train_size)
                .collect::<Vec<_>>();
            sizes.sort();
            assert_eq!(sizes, vec![104, 116, 140], "{model}");
        }
        assert!(tasks.iter().any(|task| task.kind == "production"));
        assert!(target_window_imputed(&[104, 115], 104));
        assert!(!target_window_imputed(&[103, 116], 104));
    }

    #[test]
    fn origins_keep_imputed_training_weeks() {
        // Position 50 is inside every origin's training window, never inside a
        // target window: all four origins stay planned.
        let mut series = series(152);
        series.imputed_indices = vec![50];
        let tasks = plan_tasks(&input(vec![series])).unwrap();
        let mut sizes = tasks
            .iter()
            .filter(|task| task.kind == "backtest" && task.model_id == "naive")
            .map(|task| task.train_size)
            .collect::<Vec<_>>();
        sizes.sort();
        assert_eq!(sizes, vec![104, 116, 128, 140]);
    }

    #[test]
    fn seasonal_origin_dates_align_with_the_naive_grid_for_comparable_evidence() {
        let tasks = plan_tasks(&input(vec![series(128)])).unwrap();
        let ends = |model: &str| {
            tasks
                .iter()
                .filter(|task| task.model_id == model && task.kind == "backtest")
                .map(|task| task.train_end.to_string())
                .collect::<Vec<_>>()
        };
        let naive = ends("naive");
        let seasonal = ends("seasonal_naive_52");
        // The two seasonal origins are the most recent naive origin dates.
        assert_eq!(seasonal, naive[naive.len() - 2..].to_vec());
    }

    fn fake_model(model_id: &'static str, production_eligible: bool) -> SeriesModel<'static> {
        fake_model_with_origins(model_id, production_eligible, 0)
    }

    fn fake_model_with_origins(
        model_id: &'static str,
        production_eligible: bool,
        origins: usize,
    ) -> SeriesModel<'static> {
        let start = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let completed = (0..origins)
            .map(|index| {
                (
                    start + Duration::weeks(index as i64),
                    selection::OriginSums::default(),
                )
            })
            .collect();
        SeriesModel {
            model_id,
            production_eligible,
            production: None,
            backtests: vec![],
            evidence: ModelEvidence {
                model_id: model_id.into(),
                production_eligible,
                production_completed: false,
                completed,
            },
        }
    }

    #[test]
    fn outcome_reports_model_failure_when_no_production_candidate_succeeded() {
        let models = vec![fake_model("naive", true)];
        let (outcome, reason) = outcome_for(&series(12), &models, Some(&"naive".to_string()), None);
        assert_eq!(outcome, "model_failure");
        assert!(reason.unwrap().contains("No fallback candidate"));
    }

    #[test]
    fn outcome_stays_ok_when_champion_production_failed_and_another_model_served() {
        let models = vec![fake_model("naive", true), fake_model("auto_ets_52", true)];
        let champion = "auto_ets_52".to_string();
        let used = "naive".to_string();
        let (outcome, reason) = outcome_for(&series(60), &models, Some(&champion), Some(&used));
        assert_eq!(outcome, "ok");
        assert!(reason
            .unwrap()
            .contains("Champion auto_ets_52 production failed"));
    }

    #[test]
    fn outcome_is_fallback_naive_only_without_a_champion() {
        let models = vec![fake_model("naive", true)];
        let used = "naive".to_string();
        let (outcome, reason) = outcome_for(&series(20), &models, None, Some(&used));
        assert_eq!(outcome, "fallback_naive");
        assert!(reason
            .unwrap()
            .contains("Fewer than two completed backtest origins"));
    }

    #[test]
    fn outcome_is_insufficient_history_below_every_floor() {
        let models = vec![fake_model("naive", false)];
        let (outcome, reason) = outcome_for(&series(6), &models, None, None);
        assert_eq!(outcome, "insufficient_history");
        assert!(reason.unwrap().contains("12 consecutive known weeks"));
    }

    #[test]
    fn outcome_reports_no_common_origins_when_backtests_are_disjoint() {
        let models = vec![
            fake_model_with_origins("naive", true, 2),
            fake_model_with_origins("seasonal_naive_52", true, 2),
        ];
        let used = "naive".to_string();
        let (outcome, reason) = outcome_for(&series(128), &models, None, Some(&used));
        assert_eq!(outcome, "fallback_naive");
        assert!(reason
            .unwrap()
            .contains("No common backtest origins across models"));
    }
}
