use opendemand_core::{
    forecast::run_forecast,
    types::{OverrideEdit, SeriesSummary},
    workspace::Workspace,
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

async fn planner_workspace() -> (tempfile::TempDir, Workspace, String, Vec<SeriesSummary>) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/demo.csv");
    let imported = workspace
        .import_csv(&source, &AtomicBool::new(false))
        .await
        .unwrap();
    let forecast = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "planner-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    let series = workspace.list_series(1, "", 0, 200).await.unwrap();
    (directory, workspace, forecast.run_id, series)
}

fn edit(run: &str, series: &SeriesSummary, value: Option<&str>, revision: u32) -> OverrideEdit {
    OverrideEdit {
        run_id: run.into(),
        product_id: series.product_id.clone(),
        location_id: series.location_id.clone(),
        week_start: "2026-03-02".into(),
        value: value.map(String::from),
        expected_revision: revision,
    }
}

async fn assert_consistent(workspace: &Workspace, revisions: i64) {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM forecast_override")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(count, revisions);
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
}

#[tokio::test]
async fn creating_an_override_keeps_the_baseline_and_exposes_the_effective_plan() {
    let (_directory, workspace, run, series) = planner_workspace().await;
    let series = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    let saved = workspace
        .append_override(edit(&run, series, Some("24.5"), 0))
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert!(detail.can_override);
    assert_eq!(detail.forecast[0].value, 20.0);
    assert_eq!(detail.planning.len(), 12);
    let point = &detail.planning[0];
    assert_eq!(point.week_start, "2026-03-02");
    assert_eq!(point.horizon, 1);
    assert_eq!(point.baseline, Some(20.0));
    assert_eq!(point.override_value, Some(24.5));
    assert_eq!(point.effective, Some(24.5));
    assert_eq!(point.value_source, "override");
    assert_eq!(point.override_revision, 1);
    assert_eq!(detail.planning[11].week_start, "2026-05-18");
    assert_eq!(detail.planning[11].value_source, "model");
    workspace.pool.close().await;
}

#[tokio::test]
async fn updates_and_clears_append_revisions_and_survive_reopening() {
    let (directory, workspace, run, series) = planner_workspace().await;
    let series = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    for (value, expected, revision) in [(Some("25"), 0, 1), (Some("30"), 1, 2)] {
        assert_eq!(
            workspace
                .append_override(edit(&run, series, value, expected))
                .await
                .unwrap()
                .revision,
            revision
        );
    }
    workspace.pool.close().await;
    let reopened = Workspace::open(directory.path().join("workspace"))
        .await
        .unwrap();
    let detail = reopened
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.planning[0].effective, Some(30.0));
    assert_eq!(detail.planning[0].override_revision, 2);
    assert_eq!(
        reopened
            .append_override(edit(&run, series, None, 2))
            .await
            .unwrap()
            .revision,
        3
    );
    let detail = reopened
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.forecast[0].value, 20.0);
    assert_eq!(detail.planning[0].baseline, Some(20.0));
    assert_eq!(detail.planning[0].override_value, None);
    assert_eq!(detail.planning[0].effective, Some(20.0));
    assert_eq!(detail.planning[0].value_source, "model");
    assert_eq!(detail.planning[0].override_revision, 3);
    let revisions: Vec<(i64, Option<f64>)> =
        sqlx::query_as("SELECT revision, value FROM forecast_override ORDER BY revision")
            .fetch_all(&reopened.pool)
            .await
            .unwrap();
    assert_eq!(revisions, vec![(1, Some(25.0)), (2, Some(30.0)), (3, None)]);
    assert_consistent(&reopened, 3).await;
    reopened.pool.close().await;
}

#[tokio::test]
async fn manual_only_values_can_be_created_and_cleared_back_to_missing() {
    let (_directory, workspace, run, series) = planner_workspace().await;
    let series = series
        .iter()
        .find(|row| row.series_key == "DEMAND-06")
        .unwrap();
    let error = workspace
        .append_override(edit(&run, series, None, 0))
        .await
        .unwrap_err();
    assert_eq!(error.code, "no_override");
    workspace
        .append_override(edit(&run, series, Some("0"), 0))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert!(detail.forecast.is_empty());
    assert_eq!(detail.outcome.as_deref(), Some("insufficient_history"));
    assert_eq!(detail.selected_model_id, None);
    assert_eq!(detail.planning[0].baseline, None);
    assert_eq!(detail.planning[0].effective, Some(0.0));
    assert_eq!(detail.planning[0].value_source, "override");
    assert_eq!(detail.planning[1].effective, None);
    workspace
        .append_override(edit(&run, series, None, 1))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.planning[0].override_revision, 2);
    assert_eq!(detail.planning[0].override_value, None);
    assert_eq!(detail.planning[0].effective, None);
    assert_eq!(detail.planning[0].value_source, "missing");
    assert_consistent(&workspace, 2).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn invalid_numeric_inputs_and_dates_do_not_append_any_revisions() {
    let (_directory, workspace, run, series) = planner_workspace().await;
    let series = &series[0];
    for value in [
        "", " ", "NaN", "inf", "-inf", "Infinity", "1e309", "no", "1,5", "12 units",
    ] {
        let error = workspace
            .append_override(edit(&run, series, Some(value), 0))
            .await
            .unwrap_err();
        assert_eq!(error.code, "invalid_value", "{value}");
    }
    for (week, expected) in [
        ("2026-02-23", "outside_horizon"),
        ("2026-05-25", "outside_horizon"),
        ("2026-03-03", "invalid_week"),
        ("2026-02-30", "invalid_week"),
        ("2026-3-2", "invalid_week"),
    ] {
        let mut request = edit(&run, series, Some("5"), 0);
        request.week_start = week.into();
        assert_eq!(
            workspace.append_override(request).await.unwrap_err().code,
            expected
        );
    }
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn incorrect_runs_unscoped_series_and_non_completed_runs_are_rejected() {
    let (_directory, workspace, run, series) = planner_workspace().await;
    for (field, expected) in [
        ("run", "unknown_run"),
        ("product", "unknown_series"),
        ("location", "unknown_series"),
    ] {
        let mut request = edit(&run, &series[0], Some("8"), 0);
        let unknown = uuid::Uuid::now_v7().to_string();
        match field {
            "run" => request.run_id = unknown,
            "product" => request.product_id = unknown,
            _ => request.location_id = unknown,
        }
        assert_eq!(
            workspace.append_override(request).await.unwrap_err().code,
            expected
        );
    }
    let mut request = edit(&run, &series[0], Some("8"), 0);
    request.run_id = "not-a-run-id".into();
    assert_eq!(
        workspace.append_override(request).await.unwrap_err().code,
        "invalid_input"
    );
    sqlx::query("UPDATE forecast_run SET status = 'failed' WHERE id = ?")
        .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(
        workspace
            .append_override(edit(&run, &series[0], Some("8"), 0))
            .await
            .unwrap_err()
            .code,
        "run_not_completed"
    );
    let detail = workspace
        .series_detail(1, &series[0].product_id, &series[0].location_id, Some(&run))
        .await
        .unwrap();
    assert!(!detail.can_override);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn concurrent_and_stale_edits_do_not_overwrite_each_other() {
    let (directory, workspace, run, series) = planner_workspace().await;
    let other = Workspace::open(directory.path().join("workspace"))
        .await
        .unwrap();
    let series = &series[0];
    let (first, second) = tokio::join!(
        workspace.append_override(edit(&run, series, Some("21"), 0)),
        other.append_override(edit(&run, series, Some("22"), 0)),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let conflict = if first.is_err() {
        first.unwrap_err()
    } else {
        second.unwrap_err()
    };
    assert_eq!(conflict.code, "override_conflict");
    assert_eq!(
        workspace
            .append_override(edit(&run, series, None, 0))
            .await
            .unwrap_err()
            .code,
        "override_conflict"
    );
    assert_eq!(
        workspace
            .append_override(edit(&run, series, None, 1))
            .await
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        other
            .append_override(edit(&run, series, Some("23"), 1))
            .await
            .unwrap_err()
            .code,
        "override_conflict"
    );
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.planning[0].override_revision, 2);
    assert_eq!(detail.planning[0].value_source, "missing");
    assert_consistent(&workspace, 2).await;
    other.pool.close().await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn overrides_are_isolated_between_runs_and_accept_the_last_horizon_week() {
    let (_directory, workspace, run, series) = planner_workspace().await;
    let series = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    let mut request = edit(&run, series, Some("-2.5"), 0);
    request.week_start = "2026-05-18".into();
    workspace.append_override(request).await.unwrap();
    let next = run_forecast(
        &workspace,
        1,
        None,
        "second-planner-run",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    let detail = workspace
        .series_detail(
            1,
            &series.product_id,
            &series.location_id,
            Some(&next.run_id),
        )
        .await
        .unwrap();
    assert!(detail
        .planning
        .iter()
        .all(|point| point.override_revision == 0));
    assert_eq!(detail.planning[11].baseline, Some(20.0));
    assert_eq!(detail.planning[11].effective, Some(20.0));
    workspace
        .append_override(edit(&next.run_id, series, Some("42"), 0))
        .await
        .unwrap();
    let original = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(original.planning[0].effective, Some(20.0));
    assert_eq!(original.planning[11].effective, Some(-2.5));
    assert_eq!(original.planning[11].horizon, 12);
    assert_consistent(&workspace, 2).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn csv_exports_the_full_run_including_missing_forecasts_and_records_success() {
    let (directory, workspace, run, _series) = planner_workspace().await;
    let target = directory.path().join("plan.csv");
    let result = workspace.export_csv(&run, &target).await.unwrap();
    assert_eq!(result.row_count, 36);
    assert_eq!(result.path, target.display().to_string());
    let bytes = std::fs::read(&target).unwrap();
    assert!(
        bytes.starts_with(b"\xef\xbb\xbf"),
        "Excel can recognize UTF-8"
    );
    let mut reader = csv::Reader::from_reader(bytes.as_slice());
    assert_eq!(
        reader.headers().unwrap().iter().collect::<Vec<_>>(),
        vec![
            "run_id",
            "product_sku",
            "product_name",
            "location_code",
            "week_start",
            "horizon",
            "baseline",
            "override",
            "effective",
            "value_source",
            "series_status",
            "model_id",
        ]
    );
    let rows: Vec<_> = reader.records().map(Result::unwrap).collect();
    assert_eq!(rows.len(), 36);
    assert!(rows
        .iter()
        .all(|row| row[0] == run && &row[3] == "__default__"));
    let missing: Vec<_> = rows.iter().filter(|row| &row[1] == "DEMAND-06").collect();
    assert_eq!(missing.len(), 12);
    assert!(missing
        .iter()
        .all(|row| row[6].is_empty() && row[7].is_empty() && row[8].is_empty()));
    assert!(missing.iter().all(|row| &row[9] == "missing"
        && &row[10] == "insufficient_history"
        && row[11].is_empty()));
    let baseline = rows
        .iter()
        .find(|row| &row[1] == "DEMAND-60" && &row[5] == "1")
        .unwrap();
    assert_eq!(
        (
            &baseline[6],
            &baseline[7],
            &baseline[8],
            &baseline[9],
            &baseline[11]
        ),
        ("20", "", "20", "model", "naive")
    );
    assert_eq!(&rows[11][4], "2026-05-18");
    let record: (String, i64) =
        sqlx::query_as("SELECT relative_path, row_count FROM export_record WHERE id = ?")
            .bind(result.export_id.parse::<i64>().unwrap())
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(record.1, 36);
    assert_eq!(std::fs::read(workspace.path.join(record.0)).unwrap(), bytes);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn csv_uses_latest_overrides_and_clear_revisions_without_zero_filling() {
    let (directory, workspace, run, series) = planner_workspace().await;
    let baseline = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    let fallback = series
        .iter()
        .find(|row| row.series_key == "DEMAND-20")
        .unwrap();
    let manual = series
        .iter()
        .find(|row| row.series_key == "DEMAND-06")
        .unwrap();
    for request in [
        edit(&run, baseline, Some("25"), 0),
        edit(&run, baseline, Some("30"), 1),
        edit(&run, fallback, Some("7"), 0),
        edit(&run, fallback, None, 1),
        edit(&run, manual, Some("0"), 0),
    ] {
        workspace.append_override(request).await.unwrap();
    }
    let mut request = edit(&run, manual, Some("17"), 0);
    request.week_start = "2026-03-09".into();
    workspace.append_override(request.clone()).await.unwrap();
    request.value = None;
    request.expected_revision = 1;
    workspace.append_override(request).await.unwrap();
    let target = directory.path().join("adjusted.csv");
    workspace.export_csv(&run, &target).await.unwrap();
    let rows: Vec<_> = csv::Reader::from_path(&target)
        .unwrap()
        .records()
        .map(Result::unwrap)
        .collect();
    for (sku, baseline, override_value, effective, source, model) in [
        ("DEMAND-60", "20", "30", "30", "override", "naive"),
        ("DEMAND-20", "5", "", "5", "model", "naive"),
        ("DEMAND-06", "", "0", "0", "override", ""),
    ] {
        let row = rows
            .iter()
            .find(|row| &row[1] == sku && &row[5] == "1")
            .unwrap();
        assert_eq!(
            (&row[6], &row[7], &row[8], &row[9], &row[11]),
            (baseline, override_value, effective, source, model)
        );
    }
    let cleared = rows
        .iter()
        .find(|row| &row[1] == "DEMAND-06" && &row[5] == "2")
        .unwrap();
    assert_eq!(
        (&cleared[6], &cleared[7], &cleared[8], &cleared[9]),
        ("", "", "", "missing")
    );
    workspace
        .append_override(edit(&run, baseline, None, 2))
        .await
        .unwrap();
    let reverted = directory.path().join("reverted.csv");
    workspace.export_csv(&run, &reverted).await.unwrap();
    let reverted_rows: Vec<_> = csv::Reader::from_path(&reverted)
        .unwrap()
        .records()
        .map(Result::unwrap)
        .collect();
    let row = reverted_rows
        .iter()
        .find(|row| &row[1] == "DEMAND-60" && &row[5] == "1")
        .unwrap();
    assert_eq!(
        (&row[6], &row[7], &row[8], &row[9]),
        ("20", "", "20", "model")
    );
    assert_eq!(
        rows.iter()
            .find(|row| &row[1] == "DEMAND-60" && &row[5] == "1")
            .unwrap()[8],
        *"30"
    );
    assert_consistent(&workspace, 8).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn csv_preserves_utf8_and_escapes_commas_quotes_and_newlines() {
    let (directory, workspace, run, series) = planner_workspace().await;
    let product = uuid::Uuid::parse_str(&series[0].product_id).unwrap();
    let location = uuid::Uuid::parse_str(&series[0].location_id).unwrap();
    let sku = "SKU,\"quoted\"\n\u{0141}";
    let name = "Caf\u{00e9}, \"North\"\nSecond line";
    let code = "\u{0141}\u{00f3}d\u{017a}, \"Depot\"";
    sqlx::query("UPDATE product SET sku = ?, name = ? WHERE id = ?")
        .bind(sku)
        .bind(name)
        .bind(product.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE location SET code = ? WHERE id = ?")
        .bind(code)
        .bind(location.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    let target = directory.path().join("utf8.csv");
    workspace.export_csv(&run, &target).await.unwrap();
    let bytes = std::fs::read(&target).unwrap();
    assert!(std::str::from_utf8(&bytes).unwrap().contains("Caf\u{00e9}"));
    let rows: Vec<_> = csv::Reader::from_reader(bytes.as_slice())
        .records()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 36);
    let row = rows.iter().find(|row| &row[1] == sku).unwrap();
    assert_eq!(&row[2], name);
    assert_eq!(&row[3], code);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn csv_batches_cover_every_series_location_and_week_exactly_once() {
    let (directory, workspace, run, series) = planner_workspace().await;
    let run_id = uuid::Uuid::parse_str(&run).unwrap();
    let location = uuid::Uuid::parse_str(&series[0].location_id).unwrap();
    for index in 0..41 {
        let product = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO product (id, sku, name, created_at) VALUES (?, ?, '', '2026-10-14T00:00:00Z')")
            .bind(product.as_bytes().as_slice()).bind(format!("EXTRA-{index:03}"))
            .execute(&workspace.pool).await.unwrap();
        sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status) VALUES (?, ?, ?, 'model_failure')")
            .bind(run_id.as_bytes().as_slice()).bind(product.as_bytes().as_slice())
            .bind(location.as_bytes().as_slice()).execute(&workspace.pool).await.unwrap();
    }
    let product = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    let other_location = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO location (id, code, name, created_at) VALUES (?, 'SECOND', 'Second location', '2026-10-14T00:00:00Z')")
        .bind(other_location.as_bytes().as_slice()).execute(&workspace.pool).await.unwrap();
    sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status) VALUES (?, ?, ?, 'insufficient_history')")
        .bind(run_id.as_bytes().as_slice()).bind(uuid::Uuid::parse_str(&product.product_id).unwrap().as_bytes().as_slice())
        .bind(other_location.as_bytes().as_slice()).execute(&workspace.pool).await.unwrap();
    let mut other = product.clone();
    other.location_id = other_location.to_string();
    workspace
        .append_override(edit(&run, &other, Some("9"), 0))
        .await
        .unwrap();
    let target = directory.path().join("batched.csv");
    let result = workspace.export_csv(&run, &target).await.unwrap();
    assert_eq!(result.row_count, 540);
    let rows: Vec<_> = csv::Reader::from_path(&target)
        .unwrap()
        .records()
        .map(Result::unwrap)
        .collect();
    let keys: std::collections::BTreeSet<_> = rows
        .iter()
        .map(|row| (row[1].to_owned(), row[3].to_owned(), row[4].to_owned()))
        .collect();
    assert_eq!(keys.len(), 540);
    assert_eq!(rows.len(), 540);
    let other = rows
        .iter()
        .find(|row| &row[1] == "DEMAND-60" && &row[3] == "SECOND" && &row[5] == "1")
        .unwrap();
    assert_eq!(
        (&other[6], &other[7], &other[8], &other[9]),
        ("", "9", "9", "override")
    );
    assert!(rows
        .iter()
        .filter(|row| row[1].starts_with("EXTRA-"))
        .all(|row| row[8].is_empty() && &row[10] == "model_failure"));
    assert_consistent(&workspace, 1).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn failed_csv_publication_preserves_existing_files_and_workspace_data() {
    let (directory, workspace, run, _series) = planner_workspace().await;
    let existing = directory.path().join("existing.csv");
    std::fs::write(&existing, b"existing data").unwrap();
    assert_eq!(
        workspace
            .export_csv(&run, &existing)
            .await
            .unwrap_err()
            .code,
        "export_exists"
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"existing data");
    let protected = workspace.path.join("sources/plan.csv");
    assert_eq!(
        workspace
            .export_csv(&run, &protected)
            .await
            .unwrap_err()
            .code,
        "invalid_export_path"
    );
    assert!(!protected.exists());
    let missing_parent = directory.path().join("missing/plan.csv");
    assert!(workspace.export_csv(&run, &missing_parent).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_record")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn failed_csv_record_insert_leaves_no_output_or_partial_artifacts() {
    let (directory, workspace, run, _series) = planner_workspace().await;
    sqlx::query("CREATE TRIGGER fail_export_record BEFORE INSERT ON export_record BEGIN SELECT RAISE(ABORT, 'injected export failure'); END")
        .execute(&workspace.pool).await.unwrap();
    let target = directory.path().join("failed.csv");
    assert_eq!(
        workspace.export_csv(&run, &target).await.unwrap_err().code,
        "database_error"
    );
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(workspace.path.join("exports"))
            .unwrap()
            .count(),
        0
    );
    let record_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_record")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(record_count, 0);
    let failed_jobs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM job WHERE kind = 'export' AND status = 'failed'")
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(failed_jobs, 1);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn failed_csv_commit_removes_newly_published_files_and_rolls_back_the_record() {
    let (directory, workspace, run, _series) = planner_workspace().await;
    sqlx::raw_sql("CREATE TABLE export_commit_failure (run_id BLOB REFERENCES forecast_run (id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER fail_export_commit AFTER INSERT ON export_record BEGIN INSERT INTO export_commit_failure VALUES (zeroblob(16)); END;")
        .execute(&workspace.pool).await.unwrap();
    let target = directory.path().join("failed-commit.csv");
    assert_eq!(
        workspace.export_csv(&run, &target).await.unwrap_err().code,
        "database_error"
    );
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(workspace.path.join("exports"))
            .unwrap()
            .count(),
        0
    );
    let record_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_record")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(record_count, 0);
    let failed_jobs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM job WHERE kind = 'export' AND status = 'failed'")
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(failed_jobs, 1);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn planner_can_select_runs_pinned_to_older_import_revisions() {
    let (directory, workspace, run, series) = planner_workspace().await;
    let next_source = directory.path().join("next.csv");
    std::fs::write(
        &next_source,
        "series_key,week_start,quantity\nNEW-SKU,2026-03-02,1\n",
    )
    .unwrap();
    let imported = workspace
        .import_csv(&next_source, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(imported.revision_id, 2);
    let next = run_forecast(
        &workspace,
        2,
        None,
        "new-revision-run",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    let runs = workspace.runs_list_all().await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].id, next.run_id);
    assert_eq!(runs[0].revision_id, 2);
    assert_eq!(runs[1].id, run);
    assert_eq!(runs[1].revision_id, 1);
    let old = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    let detail = workspace
        .series_detail(
            runs[1].revision_id,
            &old.product_id,
            &old.location_id,
            Some(&run),
        )
        .await
        .unwrap();
    assert_eq!(detail.planning[0].baseline, Some(20.0));
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn csv_can_save_inside_a_fresh_workspace_exports_folder_and_records_coverage() {
    let (_directory, workspace, run, _series) = planner_workspace().await;
    let target = workspace.path.join("exports/plan.CSV");
    let result = workspace.export_csv(&run, &target).await.unwrap();
    assert_eq!(result.row_count, 36);
    let coverage: String =
        sqlx::query_scalar("SELECT coverage_json FROM export_record WHERE id = ?")
            .bind(result.export_id.parse::<i64>().unwrap())
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    let coverage: serde_json::Value = serde_json::from_str(&coverage).unwrap();
    assert_eq!(coverage["series_count"], 3);
    assert_eq!(coverage["covered_series"], 2);
    assert_eq!(coverage["coverage"].as_f64().unwrap(), 0.6666666666666666);
    assert_eq!(coverage["outcomes"]["ok"], 1);
    assert_eq!(coverage["outcomes"]["fallback_naive"], 1);
    assert_eq!(coverage["outcomes"]["insufficient_history"], 1);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn concurrent_exports_to_one_destination_publish_one_complete_file() {
    let (directory, workspace, run, _series) = planner_workspace().await;
    let target = directory.path().join("same-target.csv");
    let (first, second) = tokio::join!(
        workspace.export_csv(&run, &target),
        workspace.export_csv(&run, &target)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let error = if first.is_err() {
        first.unwrap_err()
    } else {
        second.unwrap_err()
    };
    assert_eq!(error.code, "export_exists");
    assert_eq!(
        csv::Reader::from_path(&target).unwrap().records().count(),
        36
    );
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_record")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(records, 1);
    assert_eq!(
        std::fs::read_dir(workspace.path.join("exports"))
            .unwrap()
            .count(),
        1
    );
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn csv_rejects_unknown_and_non_completed_runs_without_success_records() {
    let (directory, workspace, run, _series) = planner_workspace().await;
    let target = directory.path().join("invalid-run.csv");
    assert_eq!(
        workspace
            .export_csv(&uuid::Uuid::now_v7().to_string(), &target)
            .await
            .unwrap_err()
            .code,
        "unknown_run"
    );
    sqlx::query("UPDATE forecast_run SET status = 'failed' WHERE id = ?")
        .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(
        workspace.export_csv(&run, &target).await.unwrap_err().code,
        "run_not_completed"
    );
    assert!(!target.exists());
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_record")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(records, 0);
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}
