//! Integration tests for retrospective planner FVA and persistent-bias
//! evidence (ADR-0030). The timeline is built relative to the current week so
//! the fixture stays valid on any run date: publications with historical
//! stamps are inserted directly (the real command never backdates), while
//! confirmations, overrides, and runs go through the real APIs.

use chrono::{Datelike, Duration, NaiveDate, Utc};
use opendemand_core::{
    forecast::run_forecast,
    types::{ActualsConfirmationEdit, InboxPolicyEdit, OverrideEdit, SeriesSummary},
    workspace::Workspace,
};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

fn weeks_ending_at(end: NaiveDate, count: usize) -> Vec<String> {
    (0..count)
        .rev()
        .map(|back| {
            (end - Duration::weeks(back as i64))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect()
}

fn canonical(rows: &[(String, String, f64)]) -> String {
    let mut content = String::from("series_key,week_start,quantity\n");
    for (key, week, quantity) in rows {
        content.push_str(&format!("{key},{week},{quantity}\n"));
    }
    content
}

fn series_rows(
    key: &str,
    weeks: &[String],
    quantity: f64,
    overrides: &[(String, f64)],
) -> Vec<(String, String, f64)> {
    weeks
        .iter()
        .map(|week| {
            let value = overrides
                .iter()
                .find(|(target, _)| target == week)
                .map(|(_, value)| *value)
                .unwrap_or(quantity);
            (key.to_string(), week.clone(), value)
        })
        .collect()
}

async fn forecast(workspace: &Workspace, revision_id: i32) -> String {
    run_forecast(
        workspace,
        revision_id,
        None,
        "fva-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(std::path::PathBuf::from),
    )
    .await
    .unwrap()
    .run_id
}

fn write_csv(directory: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

#[allow(clippy::too_many_arguments)]
async fn insert_publication(
    workspace: &Workspace,
    run: &str,
    revision_id: i32,
    cutoff_week: &str,
    horizon_start_week: &str,
    high_water: i64,
    published_at: &str,
    pins: &[(String, String, String, u32)],
) -> i64 {
    let publication_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO plan_publication
         (profile_id, run_id, revision_id, cutoff_week, horizon_start_week, revision_high_water, timezone, policy_context, published_at)
         VALUES (1, ?, ?, ?, ?, ?, 'UTC', 'publication/1', ?) RETURNING id",
    )
    .bind(uuid::Uuid::parse_str(run).unwrap().as_bytes().as_slice())
    .bind(revision_id)
    .bind(cutoff_week)
    .bind(horizon_start_week)
    .bind(high_water)
    .bind(published_at)
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    for (product_id, location_id, week, revision) in pins {
        sqlx::query(
            "INSERT INTO publication_override (publication_id, run_id, product_id, location_id, week_start, revision)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(publication_id)
        .bind(uuid::Uuid::parse_str(run).unwrap().as_bytes().as_slice())
        .bind(uuid::Uuid::parse_str(product_id).unwrap().as_bytes().as_slice())
        .bind(uuid::Uuid::parse_str(location_id).unwrap().as_bytes().as_slice())
        .bind(week)
        .bind(i64::from(*revision))
        .execute(&workspace.pool)
        .await
        .unwrap();
    }
    publication_id
}

struct FvaFixture {
    _directory: tempfile::TempDir,
    workspace: Workspace,
    run3: String,
    pub1: i64,
    pub2: i64,
    pub3: i64,
}

async fn find_series(workspace: &Workspace, revision_id: i32, key: &str) -> SeriesSummary {
    workspace
        .list_series(revision_id, key, 0, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|series| series.series_key == key)
        .unwrap()
}

/// Three revisions, three runs, and three publications:
///
/// - rev1 (cutoff w1) -> run1 (ALPHA override 12 on its h=2 week) -> pub1.
/// - rev2 (cutoff w2) -> run2 -> pub2; pub3 publishes run3's cutoff today
///   (a late publication of a week that already started).
/// - rev3 (cutoff = last finished week) carries the realized actuals and the
///   confirmation; a direct observation injection makes GAMMA hindsight-marked
///   for pub1.
async fn fva_fixture() -> FvaFixture {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let today = Utc::now().date_naive();
    let current_monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let w1 = current_monday - Duration::weeks(9);
    let w2 = current_monday - Duration::weeks(6);
    let w_act = current_monday - Duration::weeks(1);
    let target1 = (w1 + Duration::weeks(2)).format("%Y-%m-%d").to_string();
    let target2 = (w2 + Duration::weeks(2)).format("%Y-%m-%d").to_string();

    // Revision 1: flat histories ending at w1.
    let weeks1 = weeks_ending_at(w1, 40);
    let mut rows = Vec::new();
    rows.extend(series_rows("ALPHA", &weeks1, 10.0, &[]));
    rows.extend(series_rows("BIAS1", &weeks1, 10.0, &[]));
    rows.extend(series_rows("GAMMA", &weeks1, 5.0, &[]));
    rows.extend(series_rows("SHORT", &weeks1[34..], 4.0, &[]));
    let first = write_csv(directory.path(), "rev1.csv", &canonical(&rows));
    let imported1 = workspace
        .import_csv(&first, &AtomicBool::new(false))
        .await
        .unwrap();
    let run1 = forecast(&workspace, imported1.revision_id).await;
    let alphas = find_series(&workspace, imported1.revision_id, "ALPHA").await;

    // The planner adjusts ALPHA's h=2 week in run1 before publishing.
    workspace
        .append_override(OverrideEdit {
            run_id: run1.clone(),
            product_id: alphas.product_id.clone(),
            location_id: alphas.location_id.clone(),
            week_start: target1.clone(),
            value: Some("12".into()),
            expected_revision: 0,
            reason_code: "customer_information".into(),
            note: None,
            review_date: None,
        })
        .await
        .unwrap();

    // Revision 2: two more weeks; baselines stay 10.
    let weeks2 = weeks_ending_at(w2, 42);
    let mut rows = Vec::new();
    rows.extend(series_rows("ALPHA", &weeks2, 10.0, &[]));
    rows.extend(series_rows("BIAS1", &weeks2, 10.0, &[]));
    rows.extend(series_rows("GAMMA", &weeks2, 5.0, &[]));
    rows.extend(series_rows("SHORT", &weeks2[36..], 4.0, &[]));
    let second = write_csv(directory.path(), "rev2.csv", &canonical(&rows));
    let imported2 = workspace
        .import_csv(&second, &AtomicBool::new(false))
        .await
        .unwrap();
    let run2 = forecast(&workspace, imported2.revision_id).await;

    // Revision 3: realized actuals. ALPHA target1 = 12, target2 = 8;
    // BIAS1 = 8 on both mature target weeks (a consistent overforecast).
    let weeks3 = weeks_ending_at(w_act, 47);
    let mut rows = Vec::new();
    rows.extend(series_rows(
        "ALPHA",
        &weeks3,
        10.0,
        &[(target1.clone(), 12.0), (target2.clone(), 8.0)],
    ));
    rows.extend(series_rows(
        "BIAS1",
        &weeks3,
        10.0,
        &[(target1.clone(), 8.0), (target2.clone(), 8.0)],
    ));
    rows.extend(series_rows("GAMMA", &weeks3, 5.0, &[]));
    rows.extend(series_rows("SHORT", &weeks3[41..], 4.0, &[]));
    let third = write_csv(directory.path(), "rev3.csv", &canonical(&rows));
    let imported3 = workspace
        .import_csv(&third, &AtomicBool::new(false))
        .await
        .unwrap();
    let run3 = forecast(&workspace, imported3.revision_id).await;

    // Publications with historical stamps: pub1 (run1), pub2 (run2), pub3
    // (run3, published today for the cutoff that ended last week).
    let pub1 = insert_publication(
        &workspace,
        &run1,
        1,
        &w1.format("%Y-%m-%d").to_string(),
        &(w1 + Duration::weeks(1)).format("%Y-%m-%d").to_string(),
        1,
        &format!("{}T12:00:00Z", (w1 + Duration::days(3)).format("%Y-%m-%d")),
        &[(
            alphas.product_id.clone(),
            alphas.location_id.clone(),
            target1.clone(),
            1,
        )],
    )
    .await;
    let pub2 = insert_publication(
        &workspace,
        &run2,
        2,
        &w2.format("%Y-%m-%d").to_string(),
        &(w2 + Duration::weeks(1)).format("%Y-%m-%d").to_string(),
        2,
        &format!("{}T12:00:00Z", (w2 + Duration::days(3)).format("%Y-%m-%d")),
        &[],
    )
    .await;
    let pub3 = insert_publication(
        &workspace,
        &run3,
        3,
        &w_act.format("%Y-%m-%d").to_string(),
        &current_monday.format("%Y-%m-%d").to_string(),
        3,
        &format!("{today}T23:00:00Z"),
        &[],
    )
    .await;

    // GAMMA actuals were already visible in rev1 for the pub1 target week:
    // direct injection models a source that delivered the week early.
    let gammas = find_series(&workspace, 1, "GAMMA").await;
    sqlx::query(
        "INSERT INTO observation (revision_id, product_id, location_id, week_start, quantity, status)
         VALUES (1, ?, ?, ?, 7.0, 'known')",
    )
    .bind(uuid::Uuid::parse_str(&gammas.product_id).unwrap().as_bytes().as_slice())
    .bind(uuid::Uuid::parse_str(&gammas.location_id).unwrap().as_bytes().as_slice())
    .bind(target1.as_str())
    .execute(&workspace.pool)
    .await
    .unwrap();

    // The planner confirms the actuals revision complete through last week.
    workspace
        .actuals_confirmation_set(ActualsConfirmationEdit {
            revision_id: imported3.revision_id,
            confirmed_through: w_act.format("%Y-%m-%d").to_string(),
        })
        .await
        .unwrap();

    FvaFixture {
        _directory: directory,
        workspace,
        run3,
        pub1,
        pub2,
        pub3,
    }
}

fn find_row<'a>(
    rows: &'a [opendemand_core::types::SeriesFvaRow],
    key: &str,
) -> Option<&'a opendemand_core::types::SeriesFvaRow> {
    rows.iter().find(|row| row.series_key == key)
}

fn close(left: Option<f64>, right: f64) -> bool {
    matches!(left, Some(value) if (value - right).abs() < 1e-9)
}

#[tokio::test]
async fn fva_scores_published_adjustments_against_mature_actuals() {
    let fixture = fva_fixture().await;
    let report = fixture.workspace.fva_report(2, "", 0, 50).await.unwrap();
    assert_eq!(report.horizon, 2);
    let confirmation = report.confirmation.expect("a confirmation exists");
    assert_eq!(confirmation.revision_id, 3);
    assert_eq!(report.publications.len(), 3);

    let alphas = find_row(&report.rows, "ALPHA").expect("ALPHA is scored");
    assert_eq!(alphas.scored, 2);
    assert_eq!(alphas.adjusted, 1);
    assert!(
        close(alphas.measures.baseline_wape, 0.2),
        "{:?}",
        alphas.measures
    );
    assert!(close(alphas.measures.effective_wape, 0.1));
    assert!(close(alphas.measures.wape_improvement_pp, 10.0));
    assert!(close(alphas.measures.baseline_mae, 2.0));
    assert!(close(alphas.measures.effective_mae, 1.0));
    assert!(close(alphas.measures.mae_improvement, 1.0));
    assert!(close(alphas.measures.baseline_bias, 0.0));
    assert!(close(alphas.measures.effective_bias, 0.1));
    assert!(!alphas.imputed_input);

    // Publication counts: pub1 improved ALPHA and left BIAS1 unchanged; SHORT
    // has no forecast to score; GAMMA is hindsight-excluded for pub1.
    let pub_one = report
        .publications
        .iter()
        .find(|row| row.publication_id == i32::try_from(fixture.pub1).unwrap())
        .unwrap();
    assert_eq!(pub_one.scored, 2);
    assert_eq!(pub_one.adjusted, 1);
    assert_eq!(pub_one.improved, 1);
    assert_eq!(pub_one.unchanged, 1);
    assert_eq!(pub_one.excluded.hindsight, 1);
    assert_eq!(pub_one.excluded.no_forecast, 1);
    assert!(pub_one.low_sample);
    let pub_two = report
        .publications
        .iter()
        .find(|row| row.publication_id == i32::try_from(fixture.pub2).unwrap())
        .unwrap();
    assert_eq!(pub_two.scored, 3);
    assert_eq!(pub_two.unchanged, 3);
    assert_eq!(pub_two.excluded.no_forecast, 1);

    // GAMMA still reports from pub2, with zero error (actual 5 = baseline 5).
    let gammas = find_row(&report.rows, "GAMMA").expect("GAMMA is scored from pub2");
    assert_eq!(gammas.scored, 1);
    assert!(close(gammas.measures.baseline_mae, 0.0));
    // SHORT has no scored points at all: no forecast in any publication.
    assert!(find_row(&report.rows, "SHORT").is_none());
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn exclusions_are_explicit_and_never_zero_filled() {
    let fixture = fva_fixture().await;
    // h=12 targets are beyond the confirmed range: future, never zero-filled.
    let report = fixture.workspace.fva_report(12, "", 0, 50).await.unwrap();
    assert!(report.rows.is_empty());
    for publication in &report.publications {
        assert_eq!(publication.scored, 0);
        assert!(
            publication.excluded.future > 0,
            "publication {} future={} exclusions={:?}",
            publication.publication_id,
            publication.excluded.future,
            publication.excluded
        );
    }
    // h=1: pub3 published the week that had already started: late_published.
    let report = fixture.workspace.fva_report(1, "", 0, 50).await.unwrap();
    let pub_three = report
        .publications
        .iter()
        .find(|row| row.publication_id == i32::try_from(fixture.pub3).unwrap())
        .unwrap();
    assert_eq!(pub_three.scored, 0);
    assert_eq!(pub_three.excluded.late_published, 4);
    // The older publications still score their h=1 weeks.
    let alphas = find_row(&report.rows, "ALPHA").unwrap();
    assert_eq!(alphas.scored, 2);
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn hindsight_points_are_excluded_from_scoring() {
    let fixture = fva_fixture().await;
    let points = fixture
        .workspace
        .fva_publication_series(i32::try_from(fixture.pub1).unwrap(), 2, "", 0, 50)
        .await
        .unwrap();
    assert_eq!(points.total, 4);
    let gammas = points
        .rows
        .iter()
        .find(|row| row.series_key == "GAMMA")
        .unwrap();
    assert_eq!(gammas.excluded.as_deref(), Some("hindsight"));
    // The scored value always comes from the confirmed revision, never from
    // the visible-early revision that triggered the hindsight exclusion.
    assert_eq!(gammas.actual, Some(5.0));
    let alphas = points
        .rows
        .iter()
        .find(|row| row.series_key == "ALPHA")
        .unwrap();
    assert_eq!(alphas.excluded, None);
    assert_eq!(alphas.actual, Some(12.0));
    assert_eq!(alphas.baseline, Some(10.0));
    assert_eq!(alphas.effective, Some(12.0));
    assert_eq!(alphas.baseline_error, Some(-2.0));
    assert_eq!(alphas.effective_error, Some(0.0));
    assert_eq!(alphas.adjustment, Some(2.0));
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn confirmation_and_report_validation_is_explicit() {
    let fixture = fva_fixture().await;
    let today = Utc::now().date_naive();
    let current_monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let error = fixture
        .workspace
        .actuals_confirmation_set(ActualsConfirmationEdit {
            revision_id: 3,
            confirmed_through: current_monday.format("%Y-%m-%d").to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "confirmation_not_finished");
    let error = fixture
        .workspace
        .actuals_confirmation_set(ActualsConfirmationEdit {
            revision_id: 3,
            confirmed_through: "2026-01-06".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_week");
    let error = fixture
        .workspace
        .fva_report(0, "", 0, 50)
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_horizon");
    let error = fixture
        .workspace
        .fva_publication_series(9999, 2, "", 0, 50)
        .await
        .unwrap_err();
    assert_eq!(error.code, "unknown_publication");
    let latest = fixture
        .workspace
        .actuals_confirmation_latest()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.revision_id, 3);
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn persistent_bias_fires_on_consistent_signs_only() {
    let fixture = fva_fixture().await;
    fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            change_abs: None,
            change_rel: None,
            bias_horizon: Some("2".into()),
            bias_window: Some("4".into()),
            bias_min: Some("2".into()),
            bias_abs: Some("0.05".into()),
        })
        .await
        .unwrap();
    // Partial bias groups are refused.
    let error = fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            change_abs: None,
            change_rel: None,
            bias_horizon: Some("2".into()),
            bias_window: None,
            bias_min: None,
            bias_abs: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_policy");
    let error = fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            change_abs: None,
            change_rel: None,
            bias_horizon: Some("2".into()),
            bias_window: Some("2".into()),
            bias_min: Some("5".into()),
            bias_abs: Some("0.05".into()),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_policy");

    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run3, "", "all", 0, 50)
        .await
        .unwrap();
    let bias1 = inbox
        .rows
        .iter()
        .find(|row| row.series_key == "BIAS1")
        .expect("BIAS1 has persistent-bias evidence");
    assert!(bias1.rules.contains(&"persistent_bias".to_string()));
    assert_eq!(
        bias1.rules.len(),
        1,
        "outcome is ok, so only the bias rule fires"
    );
    assert!(close(bias1.bias_value, 0.25), "{:?}", bias1.bias_value);
    assert_eq!(bias1.bias_weeks, Some(2));
    assert_eq!(bias1.bias_horizon, Some(2));
    // ALPHA has a zero error in one window: persistence is broken, no rule.
    if let Some(alphas) = inbox.rows.iter().find(|row| row.series_key == "ALPHA") {
        assert!(!alphas.rules.iter().any(|rule| rule == "persistent_bias"));
    }
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn review_acknowledgements_include_the_bias_token() {
    let fixture = fva_fixture().await;
    let bias1 = find_series(&fixture.workspace, 3, "BIAS1").await;
    // Without a bias policy the acknowledgement holds; enabling the rule
    // changes the evidence and reopens the series without rewriting history.
    fixture
        .workspace
        .series_review_set(opendemand_core::types::ReviewActionEdit {
            run_id: fixture.run3.clone(),
            product_id: bias1.product_id.clone(),
            location_id: bias1.location_id.clone(),
            action: "reviewed".into(),
        })
        .await
        .unwrap();
    let detail = fixture
        .workspace
        .series_detail(
            3,
            &bias1.product_id,
            &bias1.location_id,
            Some(&fixture.run3),
        )
        .await
        .unwrap();
    assert_eq!(detail.review.unwrap().status, "reviewed");
    fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            change_abs: None,
            change_rel: None,
            bias_horizon: Some("2".into()),
            bias_window: Some("4".into()),
            bias_min: Some("2".into()),
            bias_abs: Some("0.05".into()),
        })
        .await
        .unwrap();
    let detail = fixture
        .workspace
        .series_detail(
            3,
            &bias1.product_id,
            &bias1.location_id,
            Some(&fixture.run3),
        )
        .await
        .unwrap();
    assert_eq!(
        detail.review.unwrap().status,
        "unreviewed",
        "new bias evidence reopens the acknowledgement"
    );
    fixture.workspace.pool.close().await;
}
