//! Weekly planning workflow: configuration suggestion, comparison pair
//! selection, and the derived demand/forecast comparison (ADR-0022).

use opendemand_core::{
    forecast::run_forecast, preprocess::ParsingDescriptor, types::OverrideEdit,
    workspace::Workspace,
};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/architecture/contracts/examples/import")
        .join(name)
}

fn write_csv(directory: &Path, name: &str, content: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn subiekt_descriptor() -> ParsingDescriptor {
    ParsingDescriptor {
        format: Some("csv".into()),
        encoding: Some("windows-1250".into()),
        delimiter: Some(";".into()),
        header_row: Some(1),
        key_columns: Some(vec!["Kod towaru".into()]),
        date_column: Some("Data sprzedaży".into()),
        date_formats: Some(vec!["%d.%m.%Y".into()]),
        quantity_column: Some("Ilość".into()),
        ..ParsingDescriptor::default()
    }
}

fn canonical(rows: &[(&str, &str, f64)]) -> String {
    let mut content = String::from("series_key,week_start,quantity\n");
    for (key, week, quantity) in rows {
        content.push_str(&format!("{key},{week},{quantity}\n"));
    }
    content
}

/// Fourteen Monday weeks ending 2026-02-23, oldest first.
const WEEKS: [&str; 14] = [
    "2025-11-24",
    "2025-12-01",
    "2025-12-08",
    "2025-12-15",
    "2025-12-22",
    "2025-12-29",
    "2026-01-05",
    "2026-01-12",
    "2026-01-19",
    "2026-01-26",
    "2026-02-02",
    "2026-02-09",
    "2026-02-16",
    "2026-02-23",
];

fn series_rows(key: &str, weeks: &[&str], quantity: f64) -> Vec<(String, String, f64)> {
    weeks
        .iter()
        .map(|week| (key.to_string(), (*week).to_string(), quantity))
        .collect()
}

fn rows_to_content(rows: &[(String, String, f64)]) -> String {
    let mut content = String::from("series_key,week_start,quantity\n");
    for (key, week, quantity) in rows {
        content.push_str(&format!("{key},{week},{quantity}\n"));
    }
    content
}

async fn forecast(workspace: &Workspace, revision_id: i32) -> String {
    run_forecast(
        workspace,
        revision_id,
        None,
        "weekly-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap()
    .run_id
}

/// A completed run row written directly: context selection is a read model and
/// these tests do not need fitted forecasts.
async fn insert_completed_run(
    workspace: &Workspace,
    revision_id: i32,
    horizon_start: &str,
) -> String {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version)
         VALUES (?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'completed', 12, ?, 'UTC', '{}', 'test', 'test')",
    )
    .bind(id.as_bytes().as_slice())
    .bind(revision_id)
    .bind(horizon_start)
    .execute(&workspace.pool)
    .await
    .unwrap();
    id.to_string()
}

async fn insert_run_with_status(workspace: &Workspace, revision_id: i32, status: &str) -> String {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO forecast_run (id, revision_id, created_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version)
         VALUES (?, ?, '2026-01-01T00:00:00Z', ?, 12, '2026-03-02', 'UTC', '{}', 'test', 'test')",
    )
    .bind(id.as_bytes().as_slice())
    .bind(revision_id)
    .bind(status)
    .execute(&workspace.pool)
    .await
    .unwrap();
    id.to_string()
}

#[tokio::test]
async fn import_config_suggestion_requires_one_exact_descriptor_match() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();

    // A canonical revision carries no descriptor, so nothing is suggested.
    let canonical_path = write_csv(
        directory.path(),
        "canonical.csv",
        &canonical(&[("ALPHA", "2026-01-05", 5.0)]),
    );
    workspace
        .import_csv(&canonical_path, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(workspace.import_config_suggestion().await.unwrap(), None);

    let saved = workspace
        .save_import_config("Subiekt tygodniowy", &subiekt_descriptor())
        .await
        .unwrap();

    // The frozen descriptor of the latest preprocessed revision matches one
    // saved configuration exactly.
    workspace
        .import_raw(
            &example("subiekt-sprzedaz.csv"),
            &subiekt_descriptor(),
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
    assert_eq!(
        workspace.import_config_suggestion().await.unwrap(),
        Some(saved.id)
    );

    // A second configuration with the same descriptor makes the match ambiguous.
    let copy = workspace
        .save_import_config("Subiekt kopia", &subiekt_descriptor())
        .await
        .unwrap();
    assert_eq!(workspace.import_config_suggestion().await.unwrap(), None);

    // Deleting one of the two restores the unique match.
    workspace.delete_import_config(saved.id).await.unwrap();
    assert_eq!(
        workspace.import_config_suggestion().await.unwrap(),
        Some(copy.id)
    );
}

#[tokio::test]
async fn weekly_comparison_covers_demand_forecast_and_override_isolation() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();

    let mut first: Vec<(String, String, f64)> = Vec::new();
    first.extend(series_rows("ALPHA", &WEEKS, 10.0));
    first.extend(series_rows("BETA", &WEEKS, 4.0));
    first.extend(series_rows("GAMMA", &WEEKS, 7.0));
    first.extend(series_rows("EPSILON", &WEEKS[6..9], 3.0));
    let first_path = write_csv(directory.path(), "first.csv", &rows_to_content(&first));
    let revision_one = workspace
        .import_csv(&first_path, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(revision_one.revision_id, 1);
    let run_one = forecast(&workspace, revision_one.revision_id).await;

    let mut second: Vec<(String, String, f64)> = Vec::new();
    second.extend(WEEKS.iter().map(|week| {
        let quantity = if *week == "2026-01-05" { 12.0 } else { 10.0 };
        ("ALPHA".to_string(), (*week).to_string(), quantity)
    }));
    second.push(("ALPHA".to_string(), "2026-03-02".to_string(), 11.0));
    second.extend(series_rows("BETA", &WEEKS[..12], 4.0));
    second.extend(series_rows("GAMMA", &WEEKS, 7.0));
    second.extend(series_rows("DELTA", &WEEKS[6..9], 5.0));
    let second_path = write_csv(directory.path(), "second.csv", &rows_to_content(&second));
    let revision_two = workspace
        .import_csv(&second_path, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(revision_two.revision_id, 2);
    assert!(!revision_two.is_duplicate);
    let run_two = forecast(&workspace, revision_two.revision_id).await;

    // A run-scoped override on the previous run must not leak into any delta.
    let series = workspace
        .list_series(revision_two.revision_id, "ALPHA", 0, 200)
        .await
        .unwrap();
    let alpha = series.first().unwrap().clone();
    workspace
        .append_override(OverrideEdit {
            run_id: run_one.clone(),
            product_id: alpha.product_id.clone(),
            location_id: alpha.location_id.clone(),
            week_start: "2026-03-09".into(),
            value: Some("999".into()),
            expected_revision: 0,
        })
        .await
        .unwrap();

    let context = workspace.weekly_comparison_context().await.unwrap();
    let current = context.current.expect("current run");
    let previous = context.previous.expect("previous run");
    assert_eq!(current.id, run_two);
    assert_eq!(previous.id, run_one);
    assert_eq!(current.revision_id, 2);
    assert_eq!(previous.revision_id, 1);

    let changed = workspace
        .weekly_comparison_series_list(&run_two, &run_one, "", 0, 200)
        .await
        .unwrap();
    let find = |key: &str| changed.iter().find(|row| row.series_key == key).unwrap();

    let alpha_summary = find("ALPHA");
    assert_eq!(alpha_summary.presence, "both");
    assert_eq!(alpha_summary.demand_change_count, 2);
    assert!(alpha_summary.forecast_change_count >= 1);

    let beta_summary = find("BETA");
    assert_eq!(beta_summary.presence, "both");
    assert_eq!(beta_summary.demand_change_count, 2);
    assert_eq!(beta_summary.forecast_change_count, 11);

    let epsilon_summary = find("EPSILON");
    assert_eq!(epsilon_summary.presence, "previous_only");
    assert_eq!(epsilon_summary.demand_change_count, 3);
    assert_eq!(epsilon_summary.forecast_change_count, 0);

    let delta_summary = find("DELTA");
    assert_eq!(delta_summary.presence, "current_only");
    assert_eq!(delta_summary.demand_change_count, 3);
    assert_eq!(delta_summary.forecast_change_count, 0);

    let alpha_detail = workspace
        .weekly_comparison_series_detail(&run_two, &run_one, &alpha.product_id, &alpha.location_id)
        .await
        .unwrap();
    assert_eq!(alpha_detail.demand_changes.len(), 2);
    assert_eq!(alpha_detail.demand_changes[0].week_start, "2026-01-05");
    assert_eq!(alpha_detail.demand_changes[0].change_kind, "changed");
    assert_eq!(alpha_detail.demand_changes[0].previous_quantity, Some(10.0));
    assert_eq!(alpha_detail.demand_changes[0].current_quantity, Some(12.0));
    assert_eq!(alpha_detail.demand_changes[1].week_start, "2026-03-02");
    assert_eq!(alpha_detail.demand_changes[1].change_kind, "added");
    assert_eq!(alpha_detail.demand_changes[1].previous_quantity, None);
    assert_eq!(alpha_detail.demand_changes[1].current_quantity, Some(11.0));

    // Near-constant history selects the naive fallback; the run still
    // persists forecast rows for the series.
    assert_eq!(
        alpha_detail.current_outcome.as_deref(),
        Some("fallback_naive")
    );
    assert_eq!(
        alpha_detail.previous_outcome.as_deref(),
        Some("fallback_naive")
    );

    // Horizon edges are visible but never changes; common weeks carry deltas
    // computed from baselines only.
    let edge_before = alpha_detail
        .forecast_changes
        .iter()
        .find(|point| point.week_start == "2026-03-02")
        .unwrap();
    assert_eq!(edge_before.comparability, "previous_only");
    assert!(!edge_before.is_change);
    assert_eq!(edge_before.delta, None);
    let edge_after = alpha_detail
        .forecast_changes
        .iter()
        .find(|point| point.week_start == "2026-05-25")
        .unwrap();
    assert_eq!(edge_after.comparability, "current_only");
    assert!(!edge_after.is_change);
    assert_eq!(edge_after.delta, None);

    let common = alpha_detail
        .forecast_changes
        .iter()
        .find(|point| point.week_start == "2026-03-09")
        .unwrap();
    assert_eq!(common.comparability, "common");
    assert_ne!(common.previous_value, Some(999.0));
    assert_ne!(common.current_value, Some(999.0));
    assert_eq!(
        common.delta,
        Some(common.current_value.unwrap() - common.previous_value.unwrap())
    );
    assert_eq!(
        common.is_change,
        common.current_value != common.previous_value
    );

    // The list count mirrors the detail derivation exactly.
    let detail_changes = alpha_detail
        .forecast_changes
        .iter()
        .filter(|point| point.is_change)
        .count();
    assert_eq!(alpha_summary.forecast_change_count as usize, detail_changes);

    // The current run's plan shows only its own overrides.
    assert_eq!(alpha_detail.planning.len(), 12);
    assert_eq!(alpha_detail.planning[0].week_start, "2026-03-09");
    assert!(alpha_detail
        .planning
        .iter()
        .all(|point| point.override_value.is_none() && point.value_source == "model"));

    let beta_detail = workspace
        .weekly_comparison_series_detail(
            &run_two,
            &run_one,
            &beta_summary.product_id,
            &beta_summary.location_id,
        )
        .await
        .unwrap();
    assert_eq!(beta_detail.demand_changes.len(), 2);
    assert!(beta_detail
        .demand_changes
        .iter()
        .all(|point| point.change_kind == "removed"));
    assert_eq!(
        beta_detail.current_outcome.as_deref(),
        Some("insufficient_history")
    );
    let beta_counted = beta_detail
        .forecast_changes
        .iter()
        .filter(|point| point.is_change)
        .count();
    assert_eq!(beta_counted, 11);
    assert!(beta_detail
        .forecast_changes
        .iter()
        .all(|point| point.comparability == "previous_only" && point.delta.is_none()));

    // Search and pagination stay deterministic.
    let searched = workspace
        .weekly_comparison_series_list(&run_two, &run_one, "ALP", 0, 200)
        .await
        .unwrap();
    assert_eq!(searched.len(), 1);
    assert_eq!(searched[0].series_key, "ALPHA");
    let page_zero = workspace
        .weekly_comparison_series_list(&run_two, &run_one, "", 0, 1)
        .await
        .unwrap();
    let page_zero_again = workspace
        .weekly_comparison_series_list(&run_two, &run_one, "", 0, 1)
        .await
        .unwrap();
    let page_one = workspace
        .weekly_comparison_series_list(&run_two, &run_one, "", 1, 1)
        .await
        .unwrap();
    assert_eq!(page_zero[0].series_key, page_zero_again[0].series_key);
    assert_ne!(page_zero[0].series_key, page_one[0].series_key);
    assert_eq!(
        changed.first().map(|row| row.series_key.clone()),
        Some(page_zero[0].series_key.clone())
    );
    let mut sorted = changed.clone();
    sorted.sort_by(|a, b| {
        a.series_key
            .cmp(&b.series_key)
            .then_with(|| a.location_id.cmp(&b.location_id))
    });
    let order: Vec<(String, String)> = changed
        .iter()
        .map(|row| (row.series_key.clone(), row.location_id.clone()))
        .collect();
    let sorted_order: Vec<(String, String)> = sorted
        .iter()
        .map(|row| (row.series_key.clone(), row.location_id.clone()))
        .collect();
    assert_eq!(order, sorted_order);

    // The current run exports through the same path the UI uses after the
    // comparison opens.
    let export_path = directory.path().join("weekly-plan.csv");
    let exported = workspace.export_csv(&run_two, &export_path).await.unwrap();
    assert!(exported.row_count > 0);
    assert!(export_path.exists());
}

#[tokio::test]
async fn weekly_context_does_not_skip_unforecast_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();

    let rows = |value: f64| {
        canonical(&[
            ("ALPHA", "2025-11-24", value),
            ("ALPHA", "2025-12-01", value),
        ])
    };
    let revision_zero = workspace
        .import_csv(
            &write_csv(directory.path(), "r0.csv", &rows(1.0)),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
        .revision_id;
    let revision_one = workspace
        .import_csv(
            &write_csv(directory.path(), "r1.csv", &rows(2.0)),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
        .revision_id;
    let revision_two = workspace
        .import_csv(
            &write_csv(directory.path(), "r2.csv", &rows(3.0)),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
        .revision_id;
    assert_eq!((revision_zero, revision_one, revision_two), (1, 2, 3));

    let run_zero = insert_completed_run(&workspace, revision_zero, "2025-12-08").await;

    // The latest revision has no completed run, so no comparison can exist
    // and neither an older run nor a previous-revision run is substituted.
    let context = workspace.weekly_comparison_context().await.unwrap();
    assert!(context.current.is_none());
    assert!(context.previous.is_none());

    let run_two = insert_completed_run(&workspace, revision_two, "2025-12-08").await;
    let context = workspace.weekly_comparison_context().await.unwrap();
    assert_eq!(context.current.as_ref().unwrap().id, run_two);
    // Revision one has no completed run; the unforecast revision is not
    // skipped in favour of run_zero.
    assert!(context.previous.is_none());

    // A pair whose runs pin non-adjacent revisions is rejected.
    let error = workspace
        .weekly_comparison_series_list(&run_two, &run_zero, "", 0, 200)
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_adjacent_revisions");

    // Reversed or identical runs are rejected before any comparison work.
    let error = workspace
        .weekly_comparison_series_list(&run_zero, &run_two, "", 0, 200)
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_comparison_pair");

    // An incomplete run is never compared.
    let failed_run = insert_run_with_status(&workspace, revision_one, "failed").await;
    let error = workspace
        .weekly_comparison_series_list(&run_two, &failed_run, "", 0, 200)
        .await
        .unwrap_err();
    assert_eq!(error.code, "run_not_completed");

    let error = workspace
        .weekly_comparison_series_list("00000000-0000-0000-0000-000000000000", &run_two, "", 0, 200)
        .await
        .unwrap_err();
    assert_eq!(error.code, "unknown_run");
}

#[tokio::test]
async fn weekly_comparison_detail_rejects_series_outside_both_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let rows = canonical(&[("ALPHA", "2025-11-24", 1.0)]);
    let revision_one = workspace
        .import_csv(
            &write_csv(directory.path(), "r1.csv", &rows),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
        .revision_id;
    let revision_two = workspace
        .import_csv(
            &write_csv(
                directory.path(),
                "r2.csv",
                &canonical(&[("ALPHA", "2025-11-24", 2.0)]),
            ),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
        .revision_id;
    let run_one = insert_completed_run(&workspace, revision_one, "2025-12-01").await;
    let run_two = insert_completed_run(&workspace, revision_two, "2025-12-01").await;

    let error = workspace
        .weekly_comparison_series_detail(
            &run_two,
            &run_one,
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222",
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "unknown_series");

    // A single-week canonical series is insufficient history, but the detail
    // still compares demand cells and returns an empty forecast comparison.
    let series = workspace
        .list_series(revision_two, "", 0, 200)
        .await
        .unwrap();
    let alpha = series.first().unwrap();
    let detail = workspace
        .weekly_comparison_series_detail(&run_two, &run_one, &alpha.product_id, &alpha.location_id)
        .await
        .unwrap();
    assert_eq!(detail.demand_changes.len(), 1);
    assert_eq!(detail.demand_changes[0].change_kind, "changed");
    assert!(detail.forecast_changes.is_empty());
}
