use opendemand_core::{forecast::run_forecast, workspace::Workspace};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

#[tokio::test]
async fn canonical_csv_reaches_real_python_engine_and_persisted_chart_payload() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/demo.csv");
    let imported = workspace
        .import_csv(&fixture, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(imported.series_count, 3);
    assert_eq!(imported.observation_count, 86);
    assert_eq!(imported.last_week, "2026-02-23");
    let repeated = workspace
        .import_csv(&fixture, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(repeated.is_duplicate);
    assert_eq!(repeated.revision_id, imported.revision_id);

    let result = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "integration-job",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    assert_eq!(result.forecast_count, 24);
    assert_eq!(result.series_count, 3);
    assert!((result.coverage - 2.0 / 3.0).abs() < 1e-9);

    let series = workspace
        .list_series(imported.revision_id, "", 0, 200)
        .await
        .unwrap();
    for (key, outcome, value, count) in [
        ("DEMAND-60", "ok", 20.0, 12),
        ("DEMAND-20", "fallback_naive", 5.0, 12),
        ("DEMAND-06", "insufficient_history", 0.0, 0),
    ] {
        let row = series.iter().find(|row| row.series_key == key).unwrap();
        let detail = workspace
            .series_detail(
                imported.revision_id,
                &row.product_id,
                &row.location_id,
                Some(&result.run_id),
            )
            .await
            .unwrap();
        assert_eq!(detail.outcome.as_deref(), Some(outcome));
        assert_eq!(detail.forecast.len(), count);
        if count > 0 {
            assert_eq!(detail.forecast[0].week_start, "2026-03-02");
            assert_eq!(detail.forecast[11].week_start, "2026-05-18");
            assert!(detail.forecast.iter().all(|point| point.value == value));
        }
        if key == "DEMAND-60" {
            // Pooled comparable WAPE: 10 total error over a 490 actual magnitude.
            assert!((detail.selection_score.unwrap() - 1.0 / 49.0).abs() < 1e-9);
        }
    }
    let origins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM series_model_origin")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(origins, 4);
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&workspace.pool)
        .await
        .unwrap()
        .is_empty());
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    let saved_run = result.run_id;
    workspace.pool.close().await;

    let reopened = Workspace::open(directory.path().join("workspace"))
        .await
        .unwrap();
    assert_eq!(
        reopened.runs_list(imported.revision_id).await.unwrap()[0].id,
        saved_run
    );
    reopened.pool.close().await;
}

#[tokio::test]
async fn multimodel_forecast_selects_comparable_champions_and_keeps_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/multimodel.csv");
    let imported = workspace
        .import_csv(&fixture, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(imported.series_count, 3);
    assert_eq!(imported.observation_count, 330);
    let result = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "multimodel-job",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    assert_eq!(result.series_count, 3);
    assert_eq!(result.forecast_count, 36);
    assert!((result.coverage - 1.0).abs() < 1e-9);

    let series = workspace
        .list_series(imported.revision_id, "", 0, 200)
        .await
        .unwrap();
    let find = |key: &str| series.iter().find(|row| row.series_key == key).unwrap();

    // SEASONAL-160: all three models completed four comparably dated origins;
    // a seasonal model wins, and Python never selected it (Rust did).
    let seasonal = find("SEASONAL-160");
    let detail = workspace
        .series_detail(
            imported.revision_id,
            &seasonal.product_id,
            &seasonal.location_id,
            Some(&result.run_id),
        )
        .await
        .unwrap();
    assert_eq!(detail.outcome.as_deref(), Some("ok"));
    assert_eq!(detail.forecast.len(), 12);
    assert!(detail.forecast.iter().all(|point| point.value.is_finite()));
    let selected = detail.selected_model_id.clone().unwrap();
    assert!(selected == "seasonal_naive_52" || selected == "auto_ets_52");
    let models = workspace
        .series_models(&result.run_id, &seasonal.product_id, &seasonal.location_id)
        .await
        .unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["naive", "seasonal_naive_52", "auto_ets_52"]
    );
    for model in &models {
        assert_eq!(model.eligibility_status, "champion_eligible");
        assert_eq!(model.origins_completed, 4);
        assert_eq!(model.origins.len(), 4);
    }
    let champion = models.iter().find(|model| model.is_champion).unwrap();
    assert_ne!(champion.model_id, "naive");
    assert_eq!(champion.comparable_metric.as_deref(), Some("wape"));
    assert!(champion.selection_score.unwrap() < 1.0);
    let used = models.iter().find(|model| model.selected).unwrap();
    assert_eq!(used.model_id, selected);
    assert_eq!(used.selection_rank, Some(1));
    let naive = models
        .iter()
        .find(|model| model.model_id == "naive")
        .unwrap();
    assert!(naive.selection_score.unwrap() > champion.selection_score.unwrap());
    assert!(naive.wape.unwrap() > 0.0);
    assert!(champion.bias.is_some());

    // CONSTANT-140: Naive is exact on a constant series, so no model can rank
    // strictly better and the documented tie-break keeps Naive as champion.
    let constant = find("CONSTANT-140");
    let detail = workspace
        .series_detail(
            imported.revision_id,
            &constant.product_id,
            &constant.location_id,
            Some(&result.run_id),
        )
        .await
        .unwrap();
    assert_eq!(detail.outcome.as_deref(), Some("ok"));
    assert_eq!(detail.selected_model_id.as_deref(), Some("naive"));
    let models = workspace
        .series_models(&result.run_id, &constant.product_id, &constant.location_id)
        .await
        .unwrap();
    let champion = models.iter().find(|model| model.is_champion).unwrap();
    assert_eq!(champion.model_id, "naive");
    // Unequal origin availability: Naive completed four origins, the seasonal
    // models three; the comparison still only used the three shared dates.
    let naive = models
        .iter()
        .find(|model| model.model_id == "naive")
        .unwrap();
    let seasonal_model = models
        .iter()
        .find(|model| model.model_id == "seasonal_naive_52")
        .unwrap();
    assert_eq!(naive.origins_completed, 4);
    assert_eq!(seasonal_model.origins_completed, 3);
    assert!(champion.selection_score.unwrap() < 1e-9);

    // SHORT-30: a single completed origin is production-only evidence, so the
    // run falls back to Naive without inventing a champion.
    let short = find("SHORT-30");
    let detail = workspace
        .series_detail(
            imported.revision_id,
            &short.product_id,
            &short.location_id,
            Some(&result.run_id),
        )
        .await
        .unwrap();
    assert_eq!(detail.outcome.as_deref(), Some("fallback_naive"));
    assert_eq!(detail.selected_model_id.as_deref(), Some("naive"));
    assert_eq!(detail.forecast.len(), 12);
    let models = workspace
        .series_models(&result.run_id, &short.product_id, &short.location_id)
        .await
        .unwrap();
    assert!(models.iter().all(|model| !model.is_champion));
    let naive = models
        .iter()
        .find(|model| model.model_id == "naive")
        .unwrap();
    assert_eq!(naive.eligibility_status, "production_only");
    assert_eq!(naive.origins_completed, 1);
    assert_eq!(naive.origins_planned, 1);
    assert!(naive.selection_score.is_none());
    assert!(models
        .iter()
        .filter(|model| model.model_id != "naive")
        .all(|model| model.eligibility_status == "not_eligible"));

    // Exactly two champions, one per champion-bearing series, and every
    // persisted selected model matches its evaluation row.
    let champions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM series_model_evaluation WHERE is_champion = 1")
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(champions, 2);
    let mismatches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM forecast_series_status status
         WHERE status.selected_model_id IS NOT NULL AND NOT EXISTS (
             SELECT 1 FROM series_model_evaluation evaluation
             WHERE evaluation.run_id = status.run_id
               AND evaluation.product_id = status.product_id
               AND evaluation.location_id = status.location_id
               AND evaluation.model_id = status.selected_model_id)",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(mismatches, 0);
    let forecast_mismatches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM forecast f JOIN forecast_series_status s
             ON s.run_id = f.run_id AND s.product_id = f.product_id
            AND s.location_id = f.location_id
         WHERE f.selected_model_id IS NOT s.selected_model_id",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(forecast_mismatches, 0);

    // Manual overrides keep working: append one, re-read the plan, export it.
    let first_week = detail.planning[0].week_start.clone();
    let baseline = detail.planning[0].baseline.unwrap();
    let revision = workspace
        .append_override(opendemand_core::types::OverrideEdit {
            run_id: result.run_id.clone(),
            product_id: short.product_id.clone(),
            location_id: short.location_id.clone(),
            week_start: first_week.clone(),
            value: Some("123".into()),
            expected_revision: 0,
        })
        .await
        .unwrap();
    assert_eq!(revision.revision, 1);
    let detail = workspace
        .series_detail(
            imported.revision_id,
            &short.product_id,
            &short.location_id,
            Some(&result.run_id),
        )
        .await
        .unwrap();
    assert_eq!(detail.planning[0].override_value, Some(123.0));
    assert_eq!(detail.planning[0].effective, Some(123.0));
    assert_eq!(detail.planning[0].baseline, Some(baseline));
    assert_eq!(detail.planning[0].value_source, "override");

    let export_directory = tempfile::tempdir().unwrap();
    let export_path = export_directory.path().join("plan.csv");
    let exported = workspace
        .export_csv(&result.run_id, &export_path)
        .await
        .unwrap();
    assert_eq!(exported.row_count, 36);
    let text = std::fs::read_to_string(&export_path).unwrap();
    let mut overridden = 0;
    for record in csv::Reader::from_reader(text.trim_start_matches('\u{feff}').as_bytes()).records()
    {
        let record = record.unwrap();
        let row = record.iter().collect::<Vec<_>>();
        if row[1] == "SHORT-30" && row[4] == first_week {
            assert_eq!(row[7], "123");
            assert_eq!(row[8], "123");
            assert_eq!(row[9], "override");
            assert_eq!(row[11], "naive");
            overridden += 1;
        }
    }
    assert_eq!(overridden, 1);

    // A later run leaves the earlier run, its overrides, and its selection
    // evidence untouched.
    let second = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "multimodel-job-2",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    assert_ne!(second.run_id, result.run_id);
    let first_forecasts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM forecast WHERE run_id = ?")
        .bind(
            uuid::Uuid::parse_str(&result.run_id)
                .unwrap()
                .as_bytes()
                .to_vec(),
        )
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(first_forecasts, 36);
    let overrides: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM forecast_override WHERE run_id = ?")
            .bind(
                uuid::Uuid::parse_str(&result.run_id)
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
            )
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(overrides, 1);
    let second_detail = workspace
        .series_detail(
            imported.revision_id,
            &short.product_id,
            &short.location_id,
            Some(&second.run_id),
        )
        .await
        .unwrap();
    assert!(second_detail
        .planning
        .iter()
        .all(|point| point.value_source == "model" && point.override_revision == 0));

    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&workspace.pool)
        .await
        .unwrap()
        .is_empty());
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    workspace.pool.close().await;
}

#[tokio::test]
async fn cancelling_before_engine_start_does_not_persist_a_partial_run() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/demo.csv");
    let imported = workspace
        .import_csv(&fixture, &AtomicBool::new(false))
        .await
        .unwrap();
    let error = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "cancelled-job",
        Arc::new(AtomicBool::new(true)),
        Arc::new(|_, _| {}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM forecast_run")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    workspace.pool.close().await;
}

#[tokio::test]
async fn cancelling_a_running_engine_does_not_persist_a_partial_run() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/demo.csv");
    let imported = workspace
        .import_csv(&fixture, &AtomicBool::new(false))
        .await
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let error = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "running-cancel-job",
        cancel,
        // Cancel on the first progress callback, before the engine finishes:
        // the core creates cancel.flag in the job directory and the engine
        // must answer job.cancelled within the grace period.
        Arc::new(move |_, _| trigger.store(true, std::sync::atomic::Ordering::SeqCst)),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM forecast_run")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    workspace.pool.close().await;
}
