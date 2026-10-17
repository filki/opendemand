//! Acceptance tests for the planner quick wins that live in the Rust core:
//! workspace locale selection, the persistent run overview with actionable
//! exceptions, and the native XLSX handoff.
//!
//! Runs come from the real Python sidecar through the public `Workspace` API,
//! the same path `tests/planner.rs` uses. A few isolated states (a champion
//! that failed in production, an empty run, export-record failures) can only
//! exist below the public API, so those tests seed them with SQL on their own
//! throwaway workspace and never touch shared state.

use calamine::{Data, Reader};
use chrono::NaiveDate;
use opendemand_core::{
    forecast::run_forecast,
    types::{OverrideEdit, SeriesSummary},
    workspace::Workspace,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::BufReader,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

async fn demo_run() -> (tempfile::TempDir, Workspace, String, Vec<SeriesSummary>) {
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
        "planner-wins-test",
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
        reason_code: "promotion".into(),
        note: None,
        review_date: None,
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

async fn export_record(workspace: &Workspace, export_id: &str) -> (String, String, i64, String) {
    sqlx::query_as(
        "SELECT format, relative_path, row_count, coverage_json FROM export_record WHERE id = ?",
    )
    .bind(export_id.parse::<i64>().unwrap())
    .fetch_one(&workspace.pool)
    .await
    .unwrap()
}

async fn set_run_status(workspace: &Workspace, run: &str, status: &str) {
    sqlx::query("UPDATE forecast_run SET status = ? WHERE id = ?")
        .bind(status)
        .bind(uuid::Uuid::parse_str(run).unwrap().as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
}

fn csv_rows(path: &Path) -> Vec<csv::StringRecord> {
    csv::Reader::from_path(path)
        .unwrap()
        .records()
        .map(Result::unwrap)
        .collect()
}

fn open_sheet(path: &Path, name: &str) -> Vec<Vec<Data>> {
    let mut workbook: calamine::Sheets<BufReader<std::fs::File>> =
        calamine::open_workbook_auto(path).unwrap();
    workbook
        .worksheet_range(name)
        .unwrap()
        .rows()
        .map(|row| row.to_vec())
        .collect()
}

fn padded(row: &[Data], width: usize) -> Vec<Data> {
    let mut padded = row.to_vec();
    padded.resize(width, Data::Empty);
    padded
}

fn is_blank(cell: &Data) -> bool {
    matches!(cell, Data::Empty) || matches!(cell, Data::String(value) if value.is_empty())
}

fn assert_text_parity(cell: &Data, expected: &str, context: &str) {
    if expected.is_empty() {
        assert!(
            is_blank(cell),
            "{context}: expected a blank cell, found {cell:?}"
        );
    } else {
        assert_eq!(text(cell).as_deref(), Some(expected), "{context}");
    }
}

fn text(cell: &Data) -> Option<String> {
    match cell {
        Data::String(value) => Some(value.clone()),
        Data::DateTimeIso(value) => Some(value.clone()),
        Data::Int(value) => Some(value.to_string()),
        Data::Float(value) => Some(value.to_string()),
        Data::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn number(cell: &Data) -> Option<f64> {
    match cell {
        Data::Int(value) => Some(*value as f64),
        Data::Float(value) => Some(*value),
        _ => None,
    }
}

fn number_label(cell: &Data) -> Option<String> {
    number(cell).map(|value| {
        if value.fract() == 0.0 {
            format!("{}", value as i64)
        } else {
            value.to_string()
        }
    })
}

fn iso_date(cell: &Data) -> Option<String> {
    match cell {
        Data::DateTime(value) => value.as_datetime().map(|moment| moment.date().to_string()),
        Data::DateTimeIso(value) => value
            .get(..10)
            .and_then(|head| NaiveDate::parse_from_str(head, "%Y-%m-%d").ok())
            .map(|date| date.to_string()),
        _ => None,
    }
}

fn plan_row(plan: &[Vec<Data>], sku: &str, location: &str, week: &str) -> Vec<Data> {
    plan[1..]
        .iter()
        .map(|row| padded(row, 12))
        .find(|row| {
            text(&row[1]).as_deref() == Some(sku)
                && text(&row[3]).as_deref() == Some(location)
                && iso_date(&row[4]).as_deref() == Some(week)
        })
        .unwrap_or_else(|| panic!("no Plan row for {sku} {location} {week}"))
}

fn summary_map(rows: &[Vec<Data>]) -> BTreeMap<String, Data> {
    rows.iter()
        .skip(1)
        .filter_map(|row| {
            let row = padded(row, 2);
            text(&row[0]).map(|label| (label, row[1].clone()))
        })
        .collect()
}

#[tokio::test]
async fn locale_selection_persists_and_invalid_values_keep_the_previous_locale() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    assert_eq!(workspace.info().await.unwrap().locale, "en");
    for locale in ["pl", "es"] {
        let updated = workspace.set_locale(locale).await.unwrap();
        assert_eq!(updated.locale, locale);
        assert_eq!(workspace.info().await.unwrap().locale, locale);
    }
    for invalid in ["de", "", "english", "pl-PL"] {
        assert_eq!(
            workspace.set_locale(invalid).await.unwrap_err().code,
            "invalid_locale",
            "{invalid}"
        );
    }
    let stored: String = sqlx::query_scalar("SELECT locale FROM workspace_settings WHERE id = 1")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(stored, "es");
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 0);
    workspace.pool.close().await;
    let reopened = Workspace::open(directory.path().join("workspace"))
        .await
        .unwrap();
    assert_eq!(reopened.info().await.unwrap().locale, "es");
    assert_eq!(reopened.set_locale("pl").await.unwrap().locale, "pl");
    reopened.pool.close().await;
    let reopened = Workspace::open(directory.path().join("workspace"))
        .await
        .unwrap();
    assert_eq!(reopened.info().await.unwrap().locale, "pl");
    assert_consistent(&reopened, 0).await;
    reopened.pool.close().await;
}

#[tokio::test]
async fn overview_matches_the_csv_histogram_and_orders_exceptions_for_the_demo_run() {
    let (directory, workspace, run, series) = demo_run().await;
    let overview = workspace.run_overview(&run, 0, 200).await.unwrap();
    assert_eq!(overview.series_count, 3);
    assert_eq!(overview.covered_series, 2);
    assert_eq!(overview.coverage, 2.0 / 3.0);
    for (outcome, expected) in [
        ("ok", 1u32),
        ("fallback_naive", 1),
        ("insufficient_history", 1),
        ("model_failure", 0),
        ("invalid_data", 0),
    ] {
        assert_eq!(overview.outcomes.get(outcome), Some(&expected), "{outcome}");
    }
    assert_eq!(overview.exception_count, 2);
    let missing = series
        .iter()
        .find(|row| row.series_key == "DEMAND-06")
        .unwrap();
    let fallback = series
        .iter()
        .find(|row| row.series_key == "DEMAND-20")
        .unwrap();
    assert_eq!(overview.exceptions[0].series_key, "DEMAND-06");
    assert_eq!(overview.exceptions[0].location_code, "__default__");
    assert_eq!(overview.exceptions[0].product_id, missing.product_id);
    assert_eq!(overview.exceptions[0].outcome, "insufficient_history");
    assert!(overview.exceptions[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("12 consecutive"));
    assert_eq!(overview.exceptions[1].series_key, "DEMAND-20");
    assert_eq!(overview.exceptions[1].product_id, fallback.product_id);
    assert_eq!(overview.exceptions[1].outcome, "fallback_naive");
    assert!(overview.exceptions[1]
        .reason
        .as_deref()
        .unwrap()
        .contains("Naive baseline"));

    let target = directory.path().join("parity.csv");
    let exported = workspace.export_csv(&run, &target).await.unwrap();
    let (_, _, row_count, coverage_json) = export_record(&workspace, &exported.export_id).await;
    assert_eq!(row_count, 36);
    let coverage: serde_json::Value = serde_json::from_str(&coverage_json).unwrap();
    assert_eq!(coverage["series_count"].as_u64().unwrap(), 3);
    assert_eq!(coverage["covered_series"].as_u64().unwrap(), 2);
    assert_eq!(coverage["coverage"].as_f64().unwrap(), overview.coverage);
    let non_zero = overview
        .outcomes
        .values()
        .filter(|count| **count > 0)
        .count();
    assert_eq!(
        coverage["outcomes"].as_object().unwrap().len(),
        non_zero,
        "CSV coverage_json and run_overview disagree on outcomes"
    );
    for (outcome, count) in &overview.outcomes {
        if *count > 0 {
            assert_eq!(
                coverage["outcomes"][outcome].as_u64().unwrap(),
                u64::from(*count)
            );
        }
    }
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn overview_pages_seeded_exceptions_with_stable_order_and_full_run_counts() {
    let (directory, workspace, run, series) = demo_run().await;
    let run_id = uuid::Uuid::parse_str(&run).unwrap();
    let location = uuid::Uuid::parse_str(&series[0].location_id).unwrap();
    for index in 0..240 {
        let product = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO product (id, sku, name, created_at) VALUES (?, ?, '', '2026-10-14T00:00:00Z')")
            .bind(product.as_bytes().as_slice())
            .bind(format!("EXTRA-{index:03}"))
            .execute(&workspace.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status) VALUES (?, ?, ?, 'model_failure')")
            .bind(run_id.as_bytes().as_slice())
            .bind(product.as_bytes().as_slice())
            .bind(location.as_bytes().as_slice())
            .execute(&workspace.pool)
            .await
            .unwrap();
    }
    let second_product = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    let second_location = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO location (id, code, name, created_at) VALUES (?, 'SECOND', 'Second location', '2026-10-14T00:00:00Z')")
        .bind(second_location.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status) VALUES (?, ?, ?, 'insufficient_history')")
        .bind(run_id.as_bytes().as_slice())
        .bind(uuid::Uuid::parse_str(&second_product.product_id).unwrap().as_bytes().as_slice())
        .bind(second_location.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();

    let overview = workspace.run_overview(&run, 0, 10).await.unwrap();
    assert_eq!(overview.series_count, 244);
    assert_eq!(overview.covered_series, 2);
    assert_eq!(overview.coverage, 2.0 / 244.0);
    assert_eq!(overview.exception_count, 243);
    assert_eq!(overview.outcomes.get("model_failure"), Some(&240));
    assert_eq!(overview.outcomes.get("insufficient_history"), Some(&2));
    assert_eq!(overview.outcomes.get("ok"), Some(&1));
    assert_eq!(overview.outcomes.get("fallback_naive"), Some(&1));
    assert_eq!(overview.outcomes.get("invalid_data"), Some(&0));
    assert_eq!(overview.exceptions.len(), 10);
    let first_page: Vec<_> = overview
        .exceptions
        .iter()
        .map(|exception| exception.series_key.clone())
        .collect();
    let expected: Vec<_> = (0..10).map(|index| format!("EXTRA-{index:03}")).collect();
    assert_eq!(first_page, expected);
    assert!(overview
        .exceptions
        .iter()
        .all(|exception| exception.outcome == "model_failure" && exception.reason.is_none()));
    let repeated: Vec<_> = workspace
        .run_overview(&run, 0, 10)
        .await
        .unwrap()
        .exceptions
        .iter()
        .map(|exception| exception.series_key.clone())
        .collect();
    assert_eq!(first_page, repeated);

    let tail = workspace.run_overview(&run, 235, 10).await.unwrap();
    let tail_keys: Vec<_> = tail
        .exceptions
        .iter()
        .map(|exception| exception.series_key.clone())
        .collect();
    assert_eq!(
        tail_keys,
        vec![
            "EXTRA-235",
            "EXTRA-236",
            "EXTRA-237",
            "EXTRA-238",
            "EXTRA-239",
            "DEMAND-06",
            "DEMAND-60",
            "DEMAND-20"
        ]
    );
    assert_eq!(tail.exceptions[5].outcome, "insufficient_history");
    assert_eq!(tail.exceptions[5].location_code, "__default__");
    assert!(tail.exceptions[5]
        .reason
        .as_deref()
        .unwrap()
        .contains("12 consecutive"));
    assert_eq!(tail.exceptions[6].location_code, "SECOND");
    assert_eq!(tail.exceptions[6].outcome, "insufficient_history");
    assert_eq!(tail.exceptions[7].outcome, "fallback_naive");

    let last = workspace.run_overview(&run, 242, 10).await.unwrap();
    assert_eq!(last.exceptions.len(), 1);
    assert_eq!(last.exceptions[0].series_key, "DEMAND-20");
    let beyond = workspace.run_overview(&run, 243, 10).await.unwrap();
    assert!(beyond.exceptions.is_empty());
    assert_eq!(beyond.exception_count, 243);
    assert_eq!(beyond.series_count, 244);
    let clamped = workspace.run_overview(&run, 0, 500).await.unwrap();
    assert_eq!(clamped.exceptions.len(), 200);
    let single = workspace.run_overview(&run, 0, 0).await.unwrap();
    assert_eq!(single.exceptions.len(), 1);

    workspace.pool.close().await;
    let reopened = Workspace::open(directory.path().join("workspace"))
        .await
        .unwrap();
    let reloaded = reopened.run_overview(&run, 0, 10).await.unwrap();
    assert_eq!(reloaded.series_count, 244);
    assert_eq!(reloaded.exception_count, 243);
    assert_eq!(reloaded.exceptions.len(), 10);
    assert_eq!(
        reloaded
            .exceptions
            .iter()
            .map(|exception| exception.series_key.clone())
            .collect::<Vec<_>>(),
        first_page
    );
    let unknown = uuid::Uuid::now_v7().to_string();
    assert_eq!(
        reopened
            .run_overview(&unknown, 0, 20)
            .await
            .unwrap_err()
            .code,
        "unknown_run"
    );
    set_run_status(&reopened, &run, "failed").await;
    assert_eq!(
        reopened.run_overview(&run, 0, 20).await.unwrap_err().code,
        "run_not_completed"
    );
    assert_consistent(&reopened, 0).await;
    reopened.pool.close().await;
}

#[tokio::test]
async fn overview_flags_ok_with_reason_all_missing_and_empty_runs() {
    let (_directory, workspace, run, series) = demo_run().await;
    let run_id = uuid::Uuid::parse_str(&run).unwrap();
    let fallback = series
        .iter()
        .find(|row| row.series_key == "DEMAND-20")
        .unwrap();
    let reason = "Champion naive production failed; using auto_ets.";
    sqlx::query("UPDATE forecast_series_status SET status = 'ok', reason = ? WHERE run_id = ? AND product_id = ? AND location_id = ?")
        .bind(reason)
        .bind(run_id.as_bytes().as_slice())
        .bind(uuid::Uuid::parse_str(&fallback.product_id).unwrap().as_bytes().as_slice())
        .bind(uuid::Uuid::parse_str(&fallback.location_id).unwrap().as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    let overview = workspace.run_overview(&run, 0, 20).await.unwrap();
    assert_eq!(overview.covered_series, 2);
    assert_eq!(overview.coverage, 2.0 / 3.0);
    assert_eq!(overview.outcomes.get("ok"), Some(&2));
    assert_eq!(overview.outcomes.get("fallback_naive"), Some(&0));
    assert_eq!(overview.exception_count, 2);
    assert_eq!(overview.exceptions.len(), 2);
    assert_eq!(overview.exceptions[0].series_key, "DEMAND-06");
    assert_eq!(overview.exceptions[1].series_key, "DEMAND-20");
    assert_eq!(overview.exceptions[1].outcome, "ok");
    assert_eq!(overview.exceptions[1].reason.as_deref(), Some(reason));

    sqlx::query("DELETE FROM forecast WHERE run_id = ?")
        .bind(run_id.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE forecast_series_status SET status = 'insufficient_history', reason = NULL, selected_model_id = NULL WHERE run_id = ?")
        .bind(run_id.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    let missing = workspace.run_overview(&run, 0, 20).await.unwrap();
    assert_eq!(missing.series_count, 3);
    assert_eq!(missing.covered_series, 0);
    assert_eq!(missing.coverage, 0.0);
    assert_eq!(missing.outcomes.get("insufficient_history"), Some(&3));
    assert_eq!(missing.outcomes.get("ok"), Some(&0));
    assert_eq!(missing.exception_count, 3);
    let keys: Vec<_> = missing
        .exceptions
        .iter()
        .map(|exception| exception.series_key.clone())
        .collect();
    assert_eq!(keys, vec!["DEMAND-06", "DEMAND-20", "DEMAND-60"]);
    assert!(missing.exceptions.iter().all(|exception| {
        exception.outcome == "insufficient_history" && exception.reason.is_none()
    }));

    let empty_run = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO forecast_run (id, revision_id, created_at, finished_at, status, horizon_weeks, horizon_start_week, timezone, config_json, app_version, engine_version) VALUES (?, 1, '2026-10-14T00:00:00Z', '2026-10-14T00:00:00Z', 'completed', 12, '2026-03-02', 'UTC', '{}', 'test', 'test')")
        .bind(empty_run.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    let empty = workspace
        .run_overview(&empty_run.to_string(), 0, 20)
        .await
        .unwrap();
    assert_eq!(empty.series_count, 0);
    assert_eq!(empty.covered_series, 0);
    assert_eq!(empty.coverage, 0.0);
    assert_eq!(empty.exception_count, 0);
    assert!(empty.exceptions.is_empty());
    assert_eq!(empty.outcomes.len(), 5);
    assert!(empty.outcomes.values().all(|count| *count == 0));
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn xlsx_plan_sheet_matches_csv_cells_with_typed_dates_numbers_and_literal_labels() {
    let (directory, workspace, run, series) = demo_run().await;
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
    let mut week_two = edit(&run, manual, Some("17"), 0);
    week_two.week_start = "2026-03-09".into();
    workspace.append_override(week_two.clone()).await.unwrap();
    week_two.value = None;
    week_two.expected_revision = 1;
    workspace.append_override(week_two).await.unwrap();

    let name = "Caf\u{00e9} \u{0141}\u{00f3}d\u{017a}, \"North\"\nSecond line";
    let location_code = "\u{0141}\u{00f3}d\u{017a}, \"Depot\"";
    sqlx::query("UPDATE product SET sku = '0070-SKU', name = ? WHERE id = ?")
        .bind(name)
        .bind(
            uuid::Uuid::parse_str(&baseline.product_id)
                .unwrap()
                .as_bytes()
                .as_slice(),
        )
        .execute(&workspace.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE product SET sku = '=SUM(A1:A2)', name = '=1+1 Depot' WHERE id = ?")
        .bind(
            uuid::Uuid::parse_str(&fallback.product_id)
                .unwrap()
                .as_bytes()
                .as_slice(),
        )
        .execute(&workspace.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE location SET code = ? WHERE id = ?")
        .bind(location_code)
        .bind(
            uuid::Uuid::parse_str(&manual.location_id)
                .unwrap()
                .as_bytes()
                .as_slice(),
        )
        .execute(&workspace.pool)
        .await
        .unwrap();

    let csv_target = directory.path().join("plan.csv");
    let xlsx_target = directory.path().join("plan.xlsx");
    let csv_result = workspace.export_csv(&run, &csv_target).await.unwrap();
    let xlsx_result = workspace.export_xlsx(&run, &xlsx_target).await.unwrap();
    assert_eq!(xlsx_result.row_count, 36);
    assert_eq!(xlsx_result.path, xlsx_target.display().to_string());

    let csv_rows = csv_rows(&csv_target);
    assert_eq!(csv_rows.len(), 36);
    let plan = open_sheet(&xlsx_target, "Plan");
    assert_eq!(plan.len(), 37);
    let header: Vec<_> = padded(&plan[0], 12)
        .iter()
        .map(|cell| text(cell).unwrap())
        .collect();
    assert_eq!(
        header,
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

    let mut csv_map = BTreeMap::new();
    for row in &csv_rows {
        csv_map.insert(format!("{}|{}|{}", &row[1], &row[4], &row[5]), row.clone());
    }
    for row in &plan[1..] {
        let row = padded(row, 12);
        let sku = text(&row[1]).unwrap();
        let week = iso_date(&row[4])
            .unwrap_or_else(|| panic!("week_start is not a typed date: {:?}", row[4]));
        let horizon = number_label(&row[5])
            .unwrap_or_else(|| panic!("horizon is not a number: {:?}", row[5]));
        let key = format!("{sku}|{week}|{horizon}");
        let csv = csv_map
            .remove(&key)
            .unwrap_or_else(|| panic!("no CSV row for {key}"));
        assert_text_parity(&row[0], &csv[0], &key);
        assert_eq!(sku, csv[1]);
        assert_text_parity(&row[2], &csv[2], &key);
        assert_text_parity(&row[3], &csv[3], &key);
        for (cell, expected) in [(&row[6], &csv[6]), (&row[7], &csv[7]), (&row[8], &csv[8])] {
            if expected.is_empty() {
                assert!(is_blank(cell), "{key} must be blank, found {cell:?}");
            } else {
                assert_eq!(
                    number(cell),
                    Some(expected.parse::<f64>().unwrap()),
                    "{key}"
                );
            }
        }
        assert_text_parity(&row[9], &csv[9], &key);
        assert_text_parity(&row[10], &csv[10], &key);
        assert_text_parity(&row[11], &csv[11], &key);
    }
    assert!(csv_map.is_empty(), "the workbook lost CSV rows");

    for row in &plan[1..] {
        let row = padded(row, 12);
        assert!(matches!(row[4], Data::DateTime(_) | Data::DateTimeIso(_)));
        assert!(matches!(row[5], Data::Int(_) | Data::Float(_)));
        for index in [6, 7, 8] {
            assert!(
                is_blank(&row[index]) || number(&row[index]).is_some(),
                "column {index} must be numeric or blank, found {:?}",
                row[index]
            );
        }
    }

    let baseline_week_one = plan_row(&plan, "0070-SKU", location_code, "2026-03-02");
    assert_eq!(number(&baseline_week_one[6]), Some(20.0));
    assert_eq!(number(&baseline_week_one[7]), Some(30.0));
    assert_eq!(number(&baseline_week_one[8]), Some(30.0));
    assert_eq!(text(&baseline_week_one[9]).as_deref(), Some("override"));
    assert_eq!(text(&baseline_week_one[2]).as_deref(), Some(name));
    let cleared_fallback = plan_row(&plan, "=SUM(A1:A2)", location_code, "2026-03-02");
    assert_eq!(number(&cleared_fallback[6]), Some(5.0));
    assert!(
        is_blank(&cleared_fallback[7]),
        "a cleared override must stay blank, not zero"
    );
    assert_eq!(number(&cleared_fallback[8]), Some(5.0));
    assert_eq!(text(&cleared_fallback[9]).as_deref(), Some("model"));
    assert!(matches!(&cleared_fallback[1], Data::String(value) if value.as_str() == "=SUM(A1:A2)"));
    assert_eq!(text(&cleared_fallback[2]).as_deref(), Some("=1+1 Depot"));
    let manual_zero = plan_row(&plan, "DEMAND-06", location_code, "2026-03-02");
    assert!(is_blank(&manual_zero[6]));
    assert_eq!(number(&manual_zero[7]), Some(0.0));
    assert_eq!(number(&manual_zero[8]), Some(0.0));
    assert_eq!(text(&manual_zero[9]).as_deref(), Some("override"));
    assert_eq!(text(&manual_zero[3]).as_deref(), Some(location_code));
    let manual_cleared = plan_row(&plan, "DEMAND-06", location_code, "2026-03-09");
    assert!(is_blank(&manual_cleared[7]));
    assert!(is_blank(&manual_cleared[8]));
    assert_eq!(text(&manual_cleared[9]).as_deref(), Some("missing"));

    let (format, relative_path, row_count, coverage_json) =
        export_record(&workspace, &xlsx_result.export_id).await;
    assert_eq!(format, "xlsx");
    assert_eq!(row_count, 36);
    assert_eq!(
        std::fs::read(workspace.path.join(&relative_path)).unwrap(),
        std::fs::read(&xlsx_target).unwrap(),
        "the retained archive must be identical to the exported workbook"
    );
    let overview = workspace.run_overview(&run, 0, 5).await.unwrap();
    let coverage: serde_json::Value = serde_json::from_str(&coverage_json).unwrap();
    assert_eq!(
        coverage["series_count"].as_u64().unwrap(),
        u64::from(overview.series_count)
    );
    assert_eq!(
        coverage["covered_series"].as_u64().unwrap(),
        u64::from(overview.covered_series)
    );
    assert_eq!(coverage["coverage"].as_f64().unwrap(), overview.coverage);
    let records: Vec<(String, String)> =
        sqlx::query_as("SELECT format, relative_path FROM export_record ORDER BY format")
            .fetch_all(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].0, "csv");
    assert_eq!(records[1].0, "xlsx");
    for (_, path) in &records {
        assert!(workspace.path.join(path).exists());
    }
    let completed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM job WHERE kind = 'export' AND status = 'completed'",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(completed, 2);
    assert_eq!(csv_result.row_count, 36);
    assert_consistent(&workspace, 7).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn xlsx_summary_sheet_reports_identity_coverage_counts_and_champions() {
    let (directory, workspace, run, _series) = demo_run().await;
    let target = directory.path().join("summary.xlsx");
    workspace.export_xlsx(&run, &target).await.unwrap();
    let summary = open_sheet(&target, "Summary");
    let header = padded(&summary[0], 2);
    assert_eq!(text(&header[0]).as_deref(), Some("Metric"));
    assert_eq!(text(&header[1]).as_deref(), Some("Value"));
    let values = summary_map(&summary);
    let engine_version = workspace.runs_list_all().await.unwrap()[0]
        .engine_version
        .clone();
    assert_eq!(text(&values["run_id"]).as_deref(), Some(run.as_str()));
    assert_eq!(number(&values["revision_id"]), Some(1.0));
    assert_eq!(
        text(&values["horizon_start_week"]).as_deref(),
        Some("2026-03-02")
    );
    assert_eq!(number(&values["horizon_weeks"]), Some(12.0));
    assert_eq!(
        text(&values["engine_version"]).as_deref(),
        Some(engine_version.as_str())
    );
    assert_eq!(number(&values["series_count"]), Some(3.0));
    assert_eq!(number(&values["covered_series"]), Some(2.0));
    assert_eq!(number(&values["coverage"]), Some(2.0 / 3.0));
    assert_eq!(number(&values["outcome.ok"]), Some(1.0));
    assert_eq!(number(&values["outcome.fallback_naive"]), Some(1.0));
    assert_eq!(number(&values["outcome.insufficient_history"]), Some(1.0));
    assert_eq!(number(&values["outcome.model_failure"]), Some(0.0));
    assert_eq!(number(&values["outcome.invalid_data"]), Some(0.0));
    assert_eq!(number(&values["statistical_champion.naive"]), Some(1.0));
    assert_eq!(
        number(&values["statistical_champion.seasonal_naive_52"]),
        Some(0.0)
    );
    assert_eq!(
        number(&values["statistical_champion.auto_ets_52"]),
        Some(0.0)
    );
    let definition = text(&values["coverage_definition"]).unwrap();
    assert!(!definition.is_empty());
    assert!(definition.to_lowercase().contains("accuracy"));
    let overview = workspace.run_overview(&run, 0, 0).await.unwrap();
    assert_eq!(
        number(&values["series_count"]),
        Some(f64::from(overview.series_count))
    );
    assert_eq!(
        number(&values["covered_series"]),
        Some(f64::from(overview.covered_series))
    );
    assert_consistent(&workspace, 0).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn xlsx_batches_cover_every_series_week_and_second_location() {
    let (directory, workspace, run, series) = demo_run().await;
    let run_id = uuid::Uuid::parse_str(&run).unwrap();
    let location = uuid::Uuid::parse_str(&series[0].location_id).unwrap();
    for index in 0..41 {
        let product = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO product (id, sku, name, created_at) VALUES (?, ?, '', '2026-10-14T00:00:00Z')")
            .bind(product.as_bytes().as_slice())
            .bind(format!("EXTRA-{index:03}"))
            .execute(&workspace.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status) VALUES (?, ?, ?, 'model_failure')")
            .bind(run_id.as_bytes().as_slice())
            .bind(product.as_bytes().as_slice())
            .bind(location.as_bytes().as_slice())
            .execute(&workspace.pool)
            .await
            .unwrap();
    }
    let product = series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap();
    let other_location = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO location (id, code, name, created_at) VALUES (?, 'SECOND', 'Second location', '2026-10-14T00:00:00Z')")
        .bind(other_location.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO forecast_series_status (run_id, product_id, location_id, status) VALUES (?, ?, ?, 'insufficient_history')")
        .bind(run_id.as_bytes().as_slice())
        .bind(uuid::Uuid::parse_str(&product.product_id).unwrap().as_bytes().as_slice())
        .bind(other_location.as_bytes().as_slice())
        .execute(&workspace.pool)
        .await
        .unwrap();
    let mut other = product.clone();
    other.location_id = other_location.to_string();
    workspace
        .append_override(edit(&run, &other, Some("9"), 0))
        .await
        .unwrap();

    let target = directory.path().join("batched.xlsx");
    let result = workspace.export_xlsx(&run, &target).await.unwrap();
    assert_eq!(result.row_count, 540);
    let plan = open_sheet(&target, "Plan");
    assert_eq!(plan.len(), 541);
    let keys: BTreeSet<(String, String, String)> = plan[1..]
        .iter()
        .map(|row| {
            let row = padded(row, 12);
            (
                text(&row[1]).unwrap(),
                text(&row[3]).unwrap(),
                iso_date(&row[4]).unwrap(),
            )
        })
        .collect();
    assert_eq!(keys.len(), 540);
    let second = plan_row(&plan, "DEMAND-60", "SECOND", "2026-03-02");
    assert!(is_blank(&second[6]));
    assert_eq!(number(&second[7]), Some(9.0));
    assert_eq!(number(&second[8]), Some(9.0));
    assert_eq!(text(&second[9]).as_deref(), Some("override"));
    let extras: Vec<_> = plan[1..]
        .iter()
        .map(|row| padded(row, 12))
        .filter(|row| text(&row[1]).unwrap().starts_with("EXTRA-"))
        .collect();
    assert_eq!(extras.len(), 492);
    assert!(extras
        .iter()
        .all(|row| { is_blank(&row[8]) && text(&row[10]).as_deref() == Some("model_failure") }));
    let (format, _, row_count, _) = export_record(&workspace, &result.export_id).await;
    assert_eq!(format, "xlsx");
    assert_eq!(row_count, 540);
    assert_consistent(&workspace, 1).await;
    workspace.pool.close().await;
}

#[tokio::test]
async fn xlsx_export_guards_reject_bad_paths_existing_files_and_bad_runs() {
    let (directory, workspace, run, _series) = demo_run().await;
    let existing = directory.path().join("existing.xlsx");
    std::fs::write(&existing, b"existing data").unwrap();
    assert_eq!(
        workspace
            .export_xlsx(&run, &existing)
            .await
            .unwrap_err()
            .code,
        "export_exists"
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"existing data");
    let protected = workspace.path.join("sources/plan.xlsx");
    assert_eq!(
        workspace
            .export_xlsx(&run, &protected)
            .await
            .unwrap_err()
            .code,
        "invalid_export_path"
    );
    assert!(!protected.exists());
    let wrong_extension = directory.path().join("plan.csv");
    assert_eq!(
        workspace
            .export_xlsx(&run, &wrong_extension)
            .await
            .unwrap_err()
            .code,
        "invalid_export_path"
    );
    assert_eq!(
        workspace
            .export_xlsx(&run, Path::new("relative.xlsx"))
            .await
            .unwrap_err()
            .code,
        "invalid_export_path"
    );
    let missing_parent = directory.path().join("missing/plan.xlsx");
    assert!(workspace.export_xlsx(&run, &missing_parent).await.is_err());
    let target = directory.path().join("guarded.xlsx");
    assert_eq!(
        workspace
            .export_xlsx(&uuid::Uuid::now_v7().to_string(), &target)
            .await
            .unwrap_err()
            .code,
        "unknown_run"
    );
    set_run_status(&workspace, &run, "failed").await;
    assert_eq!(
        workspace.export_xlsx(&run, &target).await.unwrap_err().code,
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

#[tokio::test]
async fn failed_xlsx_record_insert_leaves_no_output_or_partial_artifacts() {
    let (directory, workspace, run, _series) = demo_run().await;
    sqlx::query("CREATE TRIGGER fail_export_record BEFORE INSERT ON export_record BEGIN SELECT RAISE(ABORT, 'injected export failure'); END")
        .execute(&workspace.pool)
        .await
        .unwrap();
    let target = directory.path().join("failed.xlsx");
    assert_eq!(
        workspace.export_xlsx(&run, &target).await.unwrap_err().code,
        "database_error"
    );
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(workspace.path.join("exports"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        0
    );
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_record")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(records, 0);
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
async fn failed_xlsx_commit_removes_newly_published_files_and_rolls_back_the_record() {
    let (directory, workspace, run, _series) = demo_run().await;
    sqlx::raw_sql("CREATE TABLE export_commit_failure (run_id BLOB REFERENCES forecast_run (id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER fail_export_commit AFTER INSERT ON export_record BEGIN INSERT INTO export_commit_failure VALUES (zeroblob(16)); END;")
        .execute(&workspace.pool)
        .await
        .unwrap();
    let target = directory.path().join("failed-commit.xlsx");
    assert_eq!(
        workspace.export_xlsx(&run, &target).await.unwrap_err().code,
        "database_error"
    );
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(workspace.path.join("exports"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        0
    );
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM export_record")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(records, 0);
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
async fn concurrent_xlsx_exports_to_one_destination_publish_one_complete_workbook() {
    let (directory, workspace, run, _series) = demo_run().await;
    let target = directory.path().join("same-target.xlsx");
    let (first, second) = tokio::join!(
        workspace.export_xlsx(&run, &target),
        workspace.export_xlsx(&run, &target)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let error = if first.is_err() {
        first.unwrap_err()
    } else {
        second.unwrap_err()
    };
    assert_eq!(error.code, "export_exists");
    let plan = open_sheet(&target, "Plan");
    assert_eq!(plan.len(), 37);
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
