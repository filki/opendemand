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
        "integration-job",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        None,
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
            assert!((detail.selection_score.unwrap() - 1.0 / 52.0).abs() < 1e-9);
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
