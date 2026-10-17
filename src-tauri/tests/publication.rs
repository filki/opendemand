//! Integration tests for immutable plan publications (ADR-0029): exact
//! override pins, one publication per cutoff, the finished-week guard, and
//! exports that read pinned values and link back to the publication.

use calamine::Reader;
use chrono::Datelike;
use opendemand_core::{
    forecast::run_forecast,
    types::{OverrideEdit, PublicationEdit, SeriesSummary},
    workspace::Workspace,
};
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

async fn publication_workspace() -> (tempfile::TempDir, Workspace, String, Vec<SeriesSummary>) {
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
        "publication-test",
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
) -> OverrideEdit {
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

fn csv_rows(path: &Path) -> Vec<csv::StringRecord> {
    csv::Reader::from_path(path)
        .unwrap()
        .records()
        .map(Result::unwrap)
        .collect()
}

#[tokio::test]
async fn publishing_a_historical_run_pins_values_and_blocks_duplicates() {
    let (directory, workspace, run, series) = publication_workspace().await;
    let series = demand_series(&series);
    workspace
        .append_override(override_edit(&run, series, Some("24.5"), 0))
        .await
        .unwrap();
    let publication = workspace
        .publish_plan(PublicationEdit {
            run_id: run.clone(),
        })
        .await
        .unwrap();
    assert_eq!(publication.run_id, run);
    assert_eq!(publication.revision_id, 1);
    assert_eq!(publication.cutoff_week, "2026-02-23");
    assert_eq!(publication.horizon_start_week, "2026-03-02");
    assert_eq!(publication.revision_high_water, 1);
    assert_eq!(publication.override_cells, 1);
    assert_eq!(publication.override_series, 1);
    assert!(publication.series_count > 0);
    assert!(publication.covered_series <= publication.series_count);
    assert!(!publication.published_at.is_empty());

    let list = workspace.publications_list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, publication.id);
    assert_eq!(list[0].override_cells, 1);

    // The same cutoff cannot be published twice, not even from another run of
    // the same revision.
    let second_run = run_forecast(
        &workspace,
        1,
        None,
        "publication-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    let error = workspace
        .publish_plan(PublicationEdit {
            run_id: second_run.run_id,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "already_published");
    workspace.pool.close().await;
    drop(directory);
}

#[tokio::test]
async fn published_values_stay_pinned_after_later_edits() {
    let (directory, workspace, run, series) = publication_workspace().await;
    let series = demand_series(&series);
    workspace
        .append_override(override_edit(&run, series, Some("24.5"), 0))
        .await
        .unwrap();
    let publication = workspace
        .publish_plan(PublicationEdit {
            run_id: run.clone(),
        })
        .await
        .unwrap();
    // Later edits (a new value and then a clear) never touch the publication.
    workspace
        .append_override(override_edit(&run, series, Some("30"), 1))
        .await
        .unwrap();
    workspace
        .append_override(override_edit(&run, series, None, 2))
        .await
        .unwrap();

    let published = workspace
        .publication_series_detail(publication.id, &series.product_id, &series.location_id)
        .await
        .unwrap();
    let point = &published.planning[0];
    assert_eq!(point.week_start, "2026-03-02");
    assert_eq!(point.baseline, Some(20.0));
    assert_eq!(point.override_value, Some(24.5));
    assert_eq!(point.effective, Some(24.5));
    assert_eq!(point.override_revision, 1);
    assert_eq!(point.override_reason.as_deref(), Some("promotion"));

    let live = workspace
        .series_detail(1, &series.product_id, &series.location_id, Some(&run))
        .await
        .unwrap();
    assert_eq!(
        live.planning[0].override_value, None,
        "the live plan is cleared"
    );
    assert_eq!(live.planning[0].override_revision, 3);
    assert_eq!(live.planning[0].effective, Some(20.0));
    workspace.pool.close().await;
    drop(directory);
}

#[tokio::test]
async fn a_pinned_clear_publishes_the_model_baseline() {
    let (directory, workspace, run, series) = publication_workspace().await;
    let series = demand_series(&series);
    workspace
        .append_override(override_edit(&run, series, Some("24.5"), 0))
        .await
        .unwrap();
    workspace
        .append_override(override_edit(&run, series, None, 1))
        .await
        .unwrap();
    let publication = workspace
        .publish_plan(PublicationEdit {
            run_id: run.clone(),
        })
        .await
        .unwrap();
    let published = workspace
        .publication_series_detail(publication.id, &series.product_id, &series.location_id)
        .await
        .unwrap();
    let point = &published.planning[0];
    assert_eq!(point.override_value, None);
    assert_eq!(point.override_revision, 2);
    assert_eq!(point.effective, Some(20.0));
    assert_eq!(point.value_source, "model");
    workspace.pool.close().await;
    drop(directory);
}

#[tokio::test]
async fn history_reaching_into_an_unfinished_week_cannot_be_published() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    // Build history that ends in the current (unfinished) week.
    let today = chrono::Utc::now().date_naive();
    let current_monday =
        today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    let mut content = String::from("series_key,week_start,quantity\n");
    for back in (0..40).rev() {
        let week = current_monday - chrono::Duration::weeks(back as i64);
        content.push_str(&format!("FUTURE-1,{week},5\n"));
    }
    let source = directory.path().join("future.csv");
    std::fs::write(&source, content).unwrap();
    let imported = workspace
        .import_csv(&source, &AtomicBool::new(false))
        .await
        .unwrap();
    let run = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "publication-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    let error = workspace
        .publish_plan(PublicationEdit { run_id: run.run_id })
        .await
        .unwrap_err();
    assert_eq!(error.code, "future_history");
    assert!(workspace.publications_list().await.unwrap().is_empty());
    workspace.pool.close().await;
}

#[tokio::test]
async fn publication_exports_pin_values_and_link_the_export_record() {
    let (directory, workspace, run, series) = publication_workspace().await;
    let series = demand_series(&series);
    workspace
        .append_override(override_edit(&run, series, Some("24.5"), 0))
        .await
        .unwrap();
    let publication = workspace
        .publish_plan(PublicationEdit {
            run_id: run.clone(),
        })
        .await
        .unwrap();
    // A later edit must not leak into the publication export.
    workspace
        .append_override(override_edit(&run, series, Some("30"), 1))
        .await
        .unwrap();

    let target = directory.path().join("published.csv");
    let result = workspace
        .export_publication_csv(publication.id, &target)
        .await
        .unwrap();
    assert!(result.row_count > 0);
    let rows = csv_rows(&target);
    assert!(rows.len() as u32 == result.row_count);
    let demand_row = rows
        .iter()
        .find(|row| &row[1] == "DEMAND-60" && &row[4] == "2026-03-02")
        .expect("the pinned DEMAND-60 row exists");
    assert_eq!(
        &demand_row[7], "24.5",
        "the export keeps the pinned override"
    );
    assert_eq!(&demand_row[8], "24.5");

    let target_xlsx = directory.path().join("published.xlsx");
    workspace
        .export_publication_xlsx(publication.id, &target_xlsx)
        .await
        .unwrap();
    let mut workbook: calamine::Sheets<BufReader<std::fs::File>> =
        calamine::open_workbook_auto(&target_xlsx).unwrap();
    let summary = workbook.worksheet_range("Summary").unwrap();
    let summary_rows: Vec<Vec<calamine::Data>> = summary.rows().map(|row| row.to_vec()).collect();
    assert_eq!(
        summary_rows[18][0],
        calamine::Data::String("publication_id".into())
    );
    match summary_rows[18][1] {
        calamine::Data::Int(value) => assert_eq!(value, i64::from(publication.id)),
        calamine::Data::Float(value) => assert_eq!(value as i64, i64::from(publication.id)),
        ref other => panic!("unexpected publication_id cell: {other:?}"),
    }

    // The export records link back to the publication; ordinary run exports
    // stay NULL.
    let linked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM export_record WHERE publication_id = ?")
            .bind(i64::from(publication.id))
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(linked, 2);
    workspace.pool.close().await;
    drop(directory);
}
