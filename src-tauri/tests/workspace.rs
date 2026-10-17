//! Integration tests for the workspace import/query seam.
//!
//! These exercise the public `Workspace` API against real temporary workspaces
//! and file-backed SQLite databases: create/open, canonical import, idempotency,
//! mapping changes, validation rollbacks, duplicate handling, trailing-history
//! gaps, retained-file recovery, and run/revision scoping.

use opendemand_core::{forecast::run_forecast, imputation::ImputationMethod, workspace::Workspace};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};
use uuid::Uuid;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name)
}

fn write_csv(directory: &Path, name: &str, content: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

async fn import(workspace: &Workspace, path: &Path) -> opendemand_core::types::ImportResult {
    workspace
        .import_csv(path, &AtomicBool::new(false))
        .await
        .unwrap()
}

#[tokio::test]
async fn create_open_info_roundtrip_and_refuses_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let workspace = Workspace::create(root.clone()).await.unwrap();
    let info = workspace.info().await.unwrap();
    assert_eq!(info.schema_version, "8");
    assert_eq!(info.latest_revision_id, None);
    assert_eq!(info.locale, "en");
    assert!(info.recovery_issues.is_empty());
    assert!(root.join("workspace.sqlite3").exists());

    // A second create over the same path must not overwrite anything.
    assert!(Workspace::create(root.clone()).await.is_err());

    workspace.pool.close().await;

    let reopened = Workspace::open(root.clone()).await.unwrap();
    assert_eq!(reopened.info().await.unwrap().schema_version, "8");
    reopened.pool.close().await;

    // Opening a path with no database is rejected.
    let missing = directory.path().join("does-not-exist");
    assert!(Workspace::open(missing).await.is_err());
}

#[tokio::test]
async fn demo_fixture_import_qualifies_series_and_aligns_cutoff() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();

    let imported = import(&workspace, &fixture("demo.csv")).await;
    assert_eq!(imported.revision_id, 1);
    assert!(!imported.is_duplicate);
    assert_eq!(imported.series_count, 3);
    assert_eq!(imported.observation_count, 86);
    assert_eq!(imported.issue_count, 0);
    assert_eq!(imported.last_week, "2026-02-23");

    let series = workspace.list_series(1, "", 0, 200).await.unwrap();
    assert_eq!(series.len(), 3);
    let expected = [
        ("DEMAND-06", 6u32, "insufficient_history"),
        ("DEMAND-20", 20, "eligible"),
        ("DEMAND-60", 60, "eligible"),
    ];
    for (index, (key, h_length, status)) in expected.iter().enumerate() {
        assert_eq!(series[index].series_key, *key);
        assert_eq!(series[index].h_length, *h_length);
        assert_eq!(series[index].qualification_status, *status);
    }

    // Search filters by SKU.
    let searched = workspace.list_series(1, "DEMAND-06", 0, 200).await.unwrap();
    assert_eq!(searched.len(), 1);
    assert_eq!(searched[0].series_key, "DEMAND-06");

    let input = workspace.forecast_input(1).await.unwrap();
    assert_eq!(input.cutoff.format("%Y-%m-%d").to_string(), "2026-02-23");
    assert_eq!(input.timezone, "UTC");
    assert_eq!(input.series.len(), 3);
    let demand_60 = input
        .series
        .iter()
        .find(|series| series.series_key == "DEMAND-60")
        .unwrap();
    assert_eq!(demand_60.h_length, 60);
    assert_eq!(demand_60.observations.len(), 60);
    // The horizon starts the Monday after the global cutoff.
    assert_eq!(
        (input.cutoff + chrono::Duration::weeks(1))
            .format("%Y-%m-%d")
            .to_string(),
        "2026-03-02"
    );

    let detail = workspace
        .series_detail(1, &series[0].product_id, &series[0].location_id, None)
        .await
        .unwrap();
    assert_eq!(detail.series_key, "DEMAND-06");
    assert_eq!(detail.history.len(), 6);
    assert!(detail.history.iter().all(|point| point.quantity.is_some()));
    assert!(detail.forecast.is_empty());

    let fk: Vec<sqlx::sqlite::SqliteRow> = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&workspace.pool)
        .await
        .unwrap();
    assert!(fk.is_empty());
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    workspace.pool.close().await;
}

#[tokio::test]
async fn reimport_same_bytes_is_a_noop() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let first = import(&workspace, &fixture("demo.csv")).await;
    let second = import(&workspace, &fixture("demo.csv")).await;
    assert!(second.is_duplicate);
    assert_eq!(second.revision_id, first.revision_id);
    assert_eq!(second.series_count, 3);
    assert_eq!(second.observation_count, 86);
    assert_eq!(second.last_week, "2026-02-23");
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 1);
    workspace.pool.close().await;
}

#[tokio::test]
async fn changed_mapping_with_identical_bytes_creates_a_new_revision() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let first = import(&workspace, &fixture("demo.csv")).await;
    assert_eq!(first.revision_id, 1);

    // Simulate an operator repointing an existing mapping at a new product.
    let alternate = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO product (id, sku, name, is_active, created_at) VALUES (?, 'ALT-06', '', 1, ?)",
    )
    .bind(alternate.as_bytes().as_slice())
    .bind("2026-10-13T00:00:00Z")
    .execute(&workspace.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE product_code_map SET product_id = ? WHERE source_code = 'DEMAND-06'")
        .bind(alternate.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();

    let second = import(&workspace, &fixture("demo.csv")).await;
    assert!(!second.is_duplicate);
    assert_eq!(second.revision_id, 2);
    assert_eq!(second.series_count, 3);

    // Both revisions are retained with identical bytes.
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 2);
    assert!(workspace.path.join("sources/1/source.csv").exists());
    assert!(workspace.path.join("sources/2/source.csv").exists());
    workspace.pool.close().await;
}

#[tokio::test]
async fn invalid_header_date_and_quantity_roll_back() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();

    let header = write_csv(
        directory.path(),
        "header.csv",
        "sku,week,qty\nA,2026-01-05,1\n",
    );
    let error = workspace
        .import_csv(&header, &AtomicBool::new(false))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_header");

    let date = write_csv(
        directory.path(),
        "date.csv",
        "series_key,week_start,quantity\nA-1,2026-01-06,5\n",
    );
    let error = workspace
        .import_csv(&date, &AtomicBool::new(false))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_date");

    let quantity = write_csv(
        directory.path(),
        "quantity.csv",
        "series_key,week_start,quantity\nA-1,2026-01-05,NaN\n",
    );
    let error = workspace
        .import_csv(&quantity, &AtomicBool::new(false))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_quantity");

    // Nothing was committed by any failed attempt.
    assert_eq!(workspace.info().await.unwrap().latest_revision_id, None);
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 0);
    workspace.pool.close().await;
}

#[tokio::test]
async fn identical_duplicates_collapse_and_conflicts_exclude_a_cell() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let csv = "series_key,week_start,quantity\n\
               A-1,2026-01-05,5\n\
               A-1,2026-01-05,5\n\
               A-1,2026-01-12,3\n\
               A-1,2026-01-12,3\n\
               A-1,2026-01-19,4\n\
               B-2,2026-01-19,7\n\
               B-2,2026-01-19,9\n";
    let path = write_csv(directory.path(), "dupes.csv", csv);
    let imported = import(&workspace, &path).await;
    assert_eq!(imported.series_count, 2);
    // Only A-1's three usable cells survive; the conflicting B-2 cell is excluded.
    assert_eq!(imported.observation_count, 3);
    assert_eq!(imported.issue_count, 3);

    let series = workspace
        .list_series(imported.revision_id, "", 0, 200)
        .await
        .unwrap();
    let a1 = series.iter().find(|row| row.series_key == "A-1").unwrap();
    let b2 = series.iter().find(|row| row.series_key == "B-2").unwrap();
    assert_eq!(a1.h_length, 3);
    assert_eq!(a1.qualification_status, "insufficient_history");
    assert_eq!(b2.h_length, 0);
    assert_eq!(b2.qualification_status, "invalid_data");

    let issues: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM import_issue WHERE revision_id = ? AND code = 'duplicate_conflict'",
    )
    .bind(imported.revision_id)
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(issues, 1);

    // The conflicted series has no usable observations.
    let detail = workspace
        .series_detail(imported.revision_id, &b2.product_id, &b2.location_id, None)
        .await
        .unwrap();
    assert!(detail.history.is_empty());
    workspace.pool.close().await;
}

#[tokio::test]
async fn missing_weeks_break_trailing_history_and_render_as_gaps() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let csv = "series_key,week_start,quantity\n\
               G-1,2026-01-05,4\n\
               G-1,2026-01-12,4\n\
               G-1,2026-01-26,4\n\
               G-1,2026-02-02,4\n";
    let path = write_csv(directory.path(), "gaps.csv", csv);
    let imported = import(&workspace, &path).await;

    let series = workspace
        .list_series(imported.revision_id, "", 0, 200)
        .await
        .unwrap();
    assert_eq!(
        series[0].h_length, 2,
        "the gap at 2026-01-19 breaks the run"
    );
    assert_eq!(series[0].qualification_status, "insufficient_history");

    let detail = workspace
        .series_detail(
            imported.revision_id,
            &series[0].product_id,
            &series[0].location_id,
            None,
        )
        .await
        .unwrap();
    // Dense from 2026-01-05 to 2026-02-02 with the missing week as a null gap.
    assert_eq!(detail.history.len(), 5);
    assert_eq!(detail.history[2].week_start, "2026-01-19");
    assert_eq!(detail.history[2].quantity, None);
    assert_eq!(detail.history[2].status, "missing");
    assert!(detail.history[0].quantity.is_some());
    workspace.pool.close().await;
}

#[tokio::test]
async fn reopening_preserves_state_and_detects_retained_file_problems() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let workspace = Workspace::create(root.clone()).await.unwrap();
    import(&workspace, &fixture("demo.csv")).await;
    workspace.pool.close().await;

    // Missing committed retained file.
    std::fs::remove_file(root.join("sources/1/source.csv")).unwrap();
    let reopened = Workspace::open(root.clone()).await.unwrap();
    assert!(reopened
        .info()
        .await
        .unwrap()
        .recovery_issues
        .iter()
        .any(|issue| issue.starts_with("missing_committed_file: sources/1/source.csv")));
    reopened.pool.close().await;

    // Hash mismatch on the committed retained file.
    std::fs::write(root.join("sources/1/source.csv"), b"tampered").unwrap();
    let reopened = Workspace::open(root.clone()).await.unwrap();
    assert!(reopened
        .info()
        .await
        .unwrap()
        .recovery_issues
        .iter()
        .any(|issue| issue.starts_with("hash_mismatch: sources/1/source.csv")));
    reopened.pool.close().await;
}

#[tokio::test]
async fn orphan_and_staging_artifacts_are_quarantined_not_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let workspace = Workspace::create(root.clone()).await.unwrap();
    import(&workspace, &fixture("demo.csv")).await;
    workspace.pool.close().await;

    std::fs::create_dir_all(root.join("sources/999")).unwrap();
    std::fs::write(root.join("sources/999/orphan.csv"), b"orphan").unwrap();
    std::fs::write(root.join("staging/leftover.part"), b"staged").unwrap();

    let reopened = Workspace::open(root.clone()).await.unwrap();
    let issues = reopened.info().await.unwrap().recovery_issues;
    assert!(issues
        .iter()
        .any(|issue| issue.starts_with("orphan_final_file:")));
    assert!(issues
        .iter()
        .any(|issue| issue.starts_with("stale_staging_file:")));

    // Recoverable artifacts move to recovery/, never silently deleted.
    assert!(!root.join("sources/999/orphan.csv").exists());
    assert!(!root.join("staging/leftover.part").exists());
    let recovered: Vec<_> = std::fs::read_dir(root.join("recovery"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(recovered.len(), 2);
    reopened.pool.close().await;
}

#[tokio::test]
async fn forecast_run_scoping_rejects_mismatches_and_unknown_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    import(&workspace, &fixture("demo.csv")).await;

    let run = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version)
         VALUES (?, 1, '2026-10-13T00:00:00Z', NULL, 'completed', 12, '2026-03-02', 'UTC', '{}', '0.0.0', 'engine/1')",
    )
    .bind(run.as_bytes().as_slice())
    .execute(&workspace.pool)
    .await
    .unwrap();

    let series = workspace.list_series(1, "", 0, 200).await.unwrap();
    let scoped = workspace
        .series_detail(
            1,
            &series[0].product_id,
            &series[0].location_id,
            Some(&run.to_string()),
        )
        .await
        .unwrap();
    assert_eq!(scoped.run_id.as_deref(), Some(run.to_string().as_str()));
    assert_eq!(scoped.horizon_start_week.as_deref(), Some("2026-03-02"));

    // A run pinned to revision 1 must not be used to read revision 2.
    let alternate = Uuid::now_v7();
    sqlx::query("INSERT INTO product (id, sku, name, is_active, created_at) VALUES (?, 'ALT-06', '', 1, '2026-10-13T00:00:00Z')")
        .bind(alternate.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE product_code_map SET product_id = ? WHERE source_code = 'DEMAND-06'")
        .bind(alternate.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    import(&workspace, &fixture("demo.csv")).await;
    let series2 = workspace.list_series(2, "", 0, 200).await.unwrap();
    let error = workspace
        .series_detail(
            2,
            &series2[0].product_id,
            &series2[0].location_id,
            Some(&run.to_string()),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "run_revision_mismatch");

    let unknown_run = Uuid::now_v7();
    assert_eq!(
        workspace
            .series_detail(
                1,
                &series[0].product_id,
                &series[0].location_id,
                Some(&unknown_run.to_string())
            )
            .await
            .unwrap_err()
            .code,
        "unknown_run"
    );

    assert_eq!(
        workspace
            .list_series(9999, "", 0, 10)
            .await
            .unwrap_err()
            .code,
        "unknown_revision"
    );
    workspace.pool.close().await;
}

#[tokio::test]
async fn imputation_fills_gaps_for_the_run_only_and_records_the_method() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();

    // 130 consecutive weeks; weeks 60..=63 never appear in the source.
    let start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let mut csv = String::from("series_key,week_start,quantity\n");
    for index in 0..130 {
        if (60..=63).contains(&index) {
            continue;
        }
        let week = start + chrono::Duration::weeks(index);
        csv.push_str(&format!("GAPPY-1,{week},{}\n", 10 + index % 7));
    }
    let path = write_csv(directory.path(), "gappy.csv", &csv);
    let imported = import(&workspace, &path).await;
    assert_eq!(imported.series_count, 1);

    // Stored semantics before the run: H is the trailing gap-free run.
    let series = workspace
        .list_series(imported.revision_id, "", 0, 10)
        .await
        .unwrap();
    assert_eq!(series.len(), 1);
    let stored_h = series[0].h_length;
    assert_eq!(stored_h, 66);

    let result = run_forecast(
        &workspace,
        imported.revision_id,
        Some(ImputationMethod::Last),
        "imputation-job",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    assert_eq!(result.forecast_count, 12);
    assert_eq!(result.coverage, 1.0);

    // The run records the method and the filled-week count for the series.
    let detail = workspace
        .series_detail(
            imported.revision_id,
            &series[0].product_id,
            &series[0].location_id,
            Some(&result.run_id),
        )
        .await
        .unwrap();
    assert_eq!(detail.imputation_method.as_deref(), Some("last"));
    assert_eq!(detail.imputed_weeks, Some(4));
    assert_eq!(detail.outcome.as_deref(), Some("ok"));

    // Filled weeks unlocked the seasonal models: the four missing weeks sit
    // outside every origin's target window, so both origins are scored.
    let models = workspace
        .series_models(
            &result.run_id,
            &series[0].product_id,
            &series[0].location_id,
        )
        .await
        .unwrap();
    let seasonal = models
        .iter()
        .find(|model| model.model_id == "seasonal_naive_52")
        .unwrap();
    assert_eq!(seasonal.origins_planned, 2);
    assert_eq!(seasonal.origins_completed, 2);

    // Stored evidence is untouched: H, the history gap, and the observations.
    let after = workspace
        .list_series(imported.revision_id, "", 0, 10)
        .await
        .unwrap();
    assert_eq!(after[0].h_length, stored_h);
    let gaps = detail
        .history
        .iter()
        .filter(|point| point.quantity.is_none())
        .count();
    assert_eq!(gaps, 4);
}
