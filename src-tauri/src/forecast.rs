use crate::{
    engine::{self, EngineOutput, Progress, Task},
    error::{AppError, AppResult},
    types::{ForecastInput, ForecastResult},
    workspace::Workspace,
};
use chrono::{Duration, Utc};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use uuid::Uuid;

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
        if series.qualification_status != "eligible" || series.h_length < 12 {
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
        let mut add_task = |kind: &str, index: Option<i32>, train_size: i32| {
            let train_end = start + Duration::weeks((train_size - 1) as i64);
            tasks.push(Task {
                id: format!(
                    "{}:{}:{kind}:{}",
                    series.product_id,
                    series.location_id,
                    index.unwrap_or(0)
                ),
                product_id: series.product_id,
                location_id: series.location_id,
                kind: kind.into(),
                origin_index: index,
                train_start: start,
                train_end,
                forecast_start: train_end + Duration::weeks(1),
                train_size,
            });
        };
        add_task("production", None, h);
        let origins = ((h - 12) / 12).clamp(0, 4);
        for index in 1..=origins {
            add_task("backtest", Some(index), h - 12 * (origins - index + 1));
        }
    }
    Ok(tasks)
}

pub async fn run_forecast(
    workspace: &Workspace,
    revision_id: i32,
    job_id: &str,
    cancel: Arc<AtomicBool>,
    progress: Progress,
) -> AppResult<ForecastResult> {
    cancelled(&cancel)?;
    progress("preparing", 0.0);
    let started_at = Utc::now().to_rfc3339();
    let input = workspace.forecast_input(revision_id).await?;
    let tasks = plan_tasks(&input)?;
    let jobs_directory = workspace.path.join("jobs");
    std::fs::create_dir_all(&jobs_directory)?;
    let temporary = tempfile::Builder::new()
        .prefix("forecast-")
        .tempdir_in(jobs_directory)?;
    let output =
        engine::execute(temporary.path(), &input, &tasks, job_id, &cancel, &progress).await?;
    cancelled(&cancel)?;
    progress("persisting", 0.9);
    persist(
        workspace,
        &input,
        &tasks,
        &output,
        &started_at,
        job_id,
        &cancel,
    )
    .await
}

async fn persist(
    workspace: &Workspace,
    input: &ForecastInput,
    tasks: &[Task],
    output: &EngineOutput,
    started_at: &str,
    job_id: &str,
    cancel: &AtomicBool,
) -> AppResult<ForecastResult> {
    let _writer = workspace.writer.lock().await;
    cancelled(cancel)?;
    let mut transaction = workspace.pool.begin().await?;
    let run_id = Uuid::now_v7();
    let finished_at = Utc::now().to_rfc3339();
    let config = serde_json::json!({"models":["naive"],"window_weeks":12,"max_origins":4,"floors":{"naive":12},"contract":"opendemand.arrow/1"}).to_string();
    sqlx::query("INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version) VALUES (?, ?, ?, ?, 'completed', 12, ?, ?, ?, ?, ?)")
        .bind(run_id.as_bytes().as_slice()).bind(input.revision_id).bind(started_at).bind(&finished_at)
        .bind((input.cutoff + Duration::weeks(1)).to_string()).bind(&input.timezone).bind(config)
        .bind(env!("CARGO_PKG_VERSION")).bind(&output.engine_version).execute(&mut *transaction).await?;
    let completed = output
        .statuses
        .values()
        .filter(|status| status.status == "completed")
        .count() as i32;
    let failed = tasks.len() as i32 - completed;
    let status = if tasks.is_empty() {
        "not_run"
    } else if failed == 0 {
        "completed"
    } else if completed == 0 {
        "failed"
    } else {
        "partial"
    };
    sqlx::query("INSERT INTO run_model (run_id, model_id, status, tasks_planned, tasks_completed, tasks_failed) VALUES (?, 'naive', ?, ?, ?, ?)")
        .bind(run_id.as_bytes().as_slice()).bind(status).bind(tasks.len() as i32).bind(completed).bind(failed).execute(&mut *transaction).await?;

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
        let scoped_tasks = series_tasks
            .get(&(series.product_id, series.location_id))
            .cloned()
            .unwrap_or_default();
        let origins = scoped_tasks
            .iter()
            .filter(|task| task.kind == "backtest")
            .collect::<Vec<_>>();
        let completed_origins = origins
            .iter()
            .filter_map(|task| output.origins.get(&task.id))
            .collect::<Vec<_>>();
        let champion = completed_origins.len() >= 2;
        let score = (!completed_origins.is_empty()).then(|| {
            completed_origins
                .iter()
                .map(|metric| metric.metric_value / completed_origins.len() as f64)
                .sum::<f64>()
        });
        let production = scoped_tasks.iter().find(|task| task.kind == "production");
        let production_status = production.map(|task| &output.statuses[&task.id]);
        let eligible = series.qualification_status == "eligible" && series.h_length >= 12;
        let eligibility = if !eligible {
            "not_eligible"
        } else if champion {
            "champion_eligible"
        } else {
            "production_only"
        };
        let succeeded = production_status.is_some_and(|status| status.status == "completed");
        sqlx::query("INSERT INTO series_model_evaluation (run_id, product_id, location_id, model_id, eligibility_status, origins_planned, origins_completed, selection_score, selection_rank, is_champion, production_status, failure_code, failure_detail) VALUES (?, ?, ?, 'naive', ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
            .bind(eligibility).bind(origins.len() as i32).bind(completed_origins.len() as i32).bind(score)
            .bind(champion.then_some(1i32)).bind(i32::from(champion)).bind(production_status.map_or("not_run", |status| status.status.as_str()))
            .bind(production_status.and_then(|status| status.failure_code.as_deref())).bind(production_status.and_then(|status| status.failure_detail.as_deref()))
            .execute(&mut *transaction).await?;

        for task in origins {
            let status = &output.statuses[&task.id];
            let metric = output.origins.get(&task.id);
            sqlx::query("INSERT INTO series_model_origin (run_id, product_id, location_id, model_id, origin_index, origin_week_start, train_size, status, wape, mae, metric_kind, metric_value, failure_code, failure_detail) VALUES (?, ?, ?, 'naive', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
                .bind(task.origin_index).bind(task.train_end.to_string()).bind(task.train_size).bind(&status.status)
                .bind(metric.and_then(|metric| metric.wape)).bind(metric.map(|metric| metric.mae)).bind(metric.map(|metric| metric.metric_kind.as_str())).bind(metric.map(|metric| metric.metric_value))
                .bind(&status.failure_code).bind(&status.failure_detail).execute(&mut *transaction).await?;
            if let Some(points) = output.points.get(&task.id) {
                for point in points {
                    sqlx::query("INSERT INTO backtest_forecast (run_id, product_id, location_id, model_id, origin_index, week_start, value) VALUES (?, ?, ?, 'naive', ?, ?, ?)")
                        .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
                        .bind(task.origin_index).bind(&point.week_start).bind(point.value).execute(&mut *transaction).await?;
                }
            }
        }
        let (outcome, reason) = if series.qualification_status == "invalid_data" {
            ("invalid_data", series.reason.clone())
        } else if !eligible {
            (
                "insufficient_history",
                Some(format!(
                    "Naive requires 12 consecutive known weeks; this series has {}.",
                    series.h_length
                )),
            )
        } else if !succeeded {
            (
                "model_failure",
                production_status
                    .and_then(|status| status.failure_detail.clone())
                    .or_else(|| Some("Naive production forecast failed.".into())),
            )
        } else if champion {
            ("ok", None)
        } else {
            (
                "fallback_naive",
                Some("Fewer than two completed backtest origins; using the Naive baseline.".into()),
            )
        };
        sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status, selected_model_id, reason) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
            .bind(outcome).bind(succeeded.then_some("naive")).bind(reason).execute(&mut *transaction).await?;
        if succeeded {
            covered_series += 1;
            for point in &output.points[&production.unwrap().id] {
                sqlx::query("INSERT INTO forecast (run_id, product_id, location_id, week_start, value, selected_model_id) VALUES (?, ?, ?, ?, ?, 'naive')")
                    .bind(run_id.as_bytes().as_slice()).bind(series.product_id.as_bytes().as_slice()).bind(series.location_id.as_bytes().as_slice())
                    .bind(&point.week_start).bind(point.value).execute(&mut *transaction).await?;
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

    #[test]
    fn naive_plan_uses_series_specific_rolling_origins_and_production_floor() {
        let input = ForecastInput {
            revision_id: 1,
            cutoff: NaiveDate::from_ymd_opt(2026, 2, 23).unwrap(),
            timezone: "UTC".into(),
            series: [60, 20, 6]
                .into_iter()
                .map(|h| InputSeries {
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
                })
                .collect(),
        };
        let tasks = plan_tasks(&input).unwrap();
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
}
