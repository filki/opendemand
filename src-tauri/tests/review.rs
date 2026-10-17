//! Integration tests for planner decision memory (ADR-0027): series notes,
//! review events with evidence fingerprints, and override metadata.
//!
//! These tests run the real sidecar through `run_forecast`, like the planner
//! suites, and assert the derived-at-read-time behavior: notes survive across
//! runs, review acknowledgements reopen when evidence changes, and legacy
//! overrides without metadata stay readable.

use opendemand_core::{
    forecast::run_forecast,
    types::{OverrideEdit, ReviewActionEdit, SeriesNoteEdit, SeriesSummary},
    workspace::Workspace,
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

async fn review_workspace() -> (tempfile::TempDir, Workspace, String, Vec<SeriesSummary>) {
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
        "review-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    let series = workspace.list_series(1, "", 0, 200).await.unwrap();
    (directory, workspace, forecast.run_id, series)
}

fn demand_series(series: &[SeriesSummary]) -> &SeriesSummary {
    series
        .iter()
        .find(|row| row.series_key == "DEMAND-60")
        .unwrap()
}

fn override_edit(
    run: &str,
    series: &SeriesSummary,
    value: Option<&str>,
    revision: u32,
    reason: &str,
    note: Option<&str>,
    review_date: Option<&str>,
) -> OverrideEdit {
    OverrideEdit {
        run_id: run.into(),
        product_id: series.product_id.clone(),
        location_id: series.location_id.clone(),
        week_start: "2026-03-02".into(),
        value: value.map(String::from),
        expected_revision: revision,
        reason_code: reason.into(),
        note: note.map(String::from),
        review_date: review_date.map(String::from),
    }
}

fn note_edit(run: &str, series: &SeriesSummary, note: &str) -> SeriesNoteEdit {
    SeriesNoteEdit {
        run_id: run.into(),
        product_id: series.product_id.clone(),
        location_id: series.location_id.clone(),
        note: note.into(),
    }
}

fn review_edit(run: &str, series: &SeriesSummary, action: &str) -> ReviewActionEdit {
    ReviewActionEdit {
        run_id: run.into(),
        product_id: series.product_id.clone(),
        location_id: series.location_id.clone(),
        action: action.into(),
    }
}

#[tokio::test]
async fn override_reason_note_and_review_date_persist_with_the_revision() {
    let (_directory, workspace, run, series) = review_workspace().await;
    let series = demand_series(&series);
    workspace
        .append_override(override_edit(
            &run,
            series,
            Some("24.5"),
            0,
            "promotion",
            Some("  Spring campaign  "),
            Some("2026-06-01"),
        ))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    let point = &detail.planning[0];
    assert_eq!(point.override_revision, 1);
    assert_eq!(point.override_reason.as_deref(), Some("promotion"));
    assert_eq!(point.override_note.as_deref(), Some("Spring campaign"));
    assert_eq!(point.override_review_date.as_deref(), Some("2026-06-01"));
    // A newer revision replaces the visible metadata, including on a clear.
    workspace
        .append_override(override_edit(
            &run,
            series,
            None,
            1,
            "data_quality",
            None,
            None,
        ))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    let point = &detail.planning[0];
    assert_eq!(point.override_revision, 2);
    assert_eq!(point.override_value, None);
    assert_eq!(point.override_reason.as_deref(), Some("data_quality"));
    assert_eq!(point.override_note, None);
    assert_eq!(point.override_review_date, None);
    workspace.pool.close().await;
}

#[tokio::test]
async fn override_reason_validation_is_explicit() {
    let (_directory, workspace, run, series) = review_workspace().await;
    let series = demand_series(&series);
    let error = workspace
        .append_override(override_edit(
            &run,
            series,
            Some("5"),
            0,
            "guess",
            None,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_reason");
    let error = workspace
        .append_override(override_edit(
            &run,
            series,
            Some("5"),
            0,
            "other",
            None,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, "note_required");
    let error = workspace
        .append_override(override_edit(
            &run,
            series,
            Some("5"),
            0,
            "other",
            Some("   "),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, "note_required");
    let error = workspace
        .append_override(override_edit(
            &run,
            series,
            Some("5"),
            0,
            "promotion",
            None,
            Some("2026-06-01T00:00:00Z"),
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_review_date");
    let error = workspace
        .append_override(override_edit(
            &run,
            series,
            Some("5"),
            0,
            "promotion",
            Some(&"x".repeat(1_001)),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_note");
    // The refusals appended nothing.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM forecast_override")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    workspace.pool.close().await;
}

#[tokio::test]
async fn legacy_overrides_without_metadata_stay_readable() {
    let (_directory, workspace, run, series) = review_workspace().await;
    let series = demand_series(&series);
    // A revision written by an older build: value present, no metadata row.
    sqlx::query(
        "INSERT INTO forecast_override (run_id, product_id, location_id, week_start, revision, value, note, created_at)
         VALUES (?, ?, ?, '2026-03-02', 1, 15.0, NULL, '2026-01-01T00:00:00Z')",
    )
    .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
    .bind(uuid::Uuid::parse_str(&series.product_id).unwrap().as_bytes().as_slice())
    .bind(uuid::Uuid::parse_str(&series.location_id).unwrap().as_bytes().as_slice())
    .execute(&workspace.pool)
    .await
    .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    let point = &detail.planning[0];
    assert_eq!(point.override_revision, 1);
    assert_eq!(point.override_value, Some(15.0));
    assert_eq!(
        point.override_reason, None,
        "legacy metadata must stay absent"
    );
    assert_eq!(point.override_note, None);
    assert_eq!(point.override_review_date, None);
    workspace.pool.close().await;
}

#[tokio::test]
async fn series_notes_survive_a_new_run_and_stay_with_the_series() {
    let (_directory, workspace, run, series) = review_workspace().await;
    let series = demand_series(&series);
    let saved = workspace
        .series_note_add(note_edit(&run, series, "Watch the weather week 12."))
        .await
        .unwrap();
    assert_eq!(saved.note, "Watch the weather week 12.");
    assert_eq!(saved.context_run_id.as_deref(), Some(run.as_str()));

    // A second completed run on the same revision: notes carry, review does not.
    let second = run_forecast(
        &workspace,
        1,
        None,
        "review-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    workspace
        .series_review_set(review_edit(&run, series, "reviewed"))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(
            1,
            &series.product_id,
            &series.location_id,
            Some(&second.run_id),
        )
        .await
        .unwrap();
    assert_eq!(detail.notes.len(), 1);
    assert_eq!(detail.notes[0].note, "Watch the weather week 12.");
    let review = detail.review.unwrap();
    assert_eq!(review.status, "unreviewed");
    assert_eq!(review.reviewed_at, None);
    workspace.pool.close().await;
}

#[tokio::test]
async fn review_acknowledgements_reopen_when_evidence_changes() {
    let (_directory, workspace, run, series) = review_workspace().await;
    let series = demand_series(&series);
    let reviewed = workspace
        .series_review_set(review_edit(&run, series, "reviewed"))
        .await
        .unwrap();
    assert_eq!(reviewed.status, "reviewed");
    assert!(reviewed.reviewed_at.is_some());
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.review.as_ref().unwrap().status, "reviewed");

    // A new override revision is new evidence: the acknowledgement reopens.
    workspace
        .append_override(override_edit(
            &run,
            series,
            Some("9"),
            0,
            "promotion",
            None,
            None,
        ))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.review.as_ref().unwrap().status, "unreviewed");
    assert_eq!(detail.review.as_ref().unwrap().reviewed_at, None);

    // Reviewing again acknowledges the new evidence.
    workspace
        .series_review_set(review_edit(&run, series, "reviewed"))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.review.as_ref().unwrap().status, "reviewed");

    // An explicit reopen clears the acknowledgement.
    workspace
        .series_review_set(review_edit(&run, series, "reopened"))
        .await
        .unwrap();
    let detail = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.review.as_ref().unwrap().status, "unreviewed");

    // The event history is append-only at the SQL level.
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM series_review")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(events, 3);
    workspace.pool.close().await;
}

#[tokio::test]
async fn decision_commands_validate_run_scope_and_actions() {
    let (_directory, workspace, run, series) = review_workspace().await;
    let series = demand_series(&series);
    let error = workspace
        .series_review_set(review_edit(&run, series, "approved"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_action");
    let error = workspace
        .series_note_add(note_edit(&run, series, "   "))
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_note");
    let unknown_run = "0198d3c1-1111-7000-8000-000000000000";
    let error = workspace
        .series_note_add(note_edit(unknown_run, series, "note"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "unknown_run");
    let unknown_series = "0198d3c1-2222-7000-8000-000000000000";
    let mut missing = note_edit(&run, series, "note");
    missing.product_id = unknown_series.into();
    let error = workspace.series_note_add(missing).await.unwrap_err();
    assert_eq!(error.code, "unknown_series");
    let mut missing = review_edit(&run, series, "reviewed");
    missing.location_id = unknown_series.into();
    let error = workspace.series_review_set(missing).await.unwrap_err();
    assert_eq!(error.code, "unknown_series");
    workspace.pool.close().await;
}

#[tokio::test]
async fn notes_and_reviews_survive_reopening() {
    let (directory, workspace, run, series) = review_workspace().await;
    let series = demand_series(&series);
    workspace
        .series_note_add(note_edit(&run, series, "Reopen-safe note."))
        .await
        .unwrap();
    workspace
        .series_review_set(review_edit(&run, series, "reviewed"))
        .await
        .unwrap();
    workspace.pool.close().await;

    let reopened = Workspace::open(directory.path().join("workspace"))
        .await
        .unwrap();
    let detail = reopened
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(detail.notes[0].note, "Reopen-safe note.");
    assert_eq!(detail.review.unwrap().status, "reviewed");
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&reopened.pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    reopened.pool.close().await;
}
