//! Integration tests for the derived exception inbox (ADR-0028): rule codes,
//! fixed priority, material-change policy thresholds, review-state filtering,
//! and due review reminders. Everything here is computed at read time.

use opendemand_core::{
    forecast::run_forecast,
    imputation::ImputationMethod,
    types::{InboxPolicyEdit, OverrideEdit, ReviewActionEdit, SeriesNoteEdit},
    workspace::Workspace,
};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

fn write_csv(directory: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// Monday weeks ending at `end`, oldest first.
fn weeks_ending(end: &str, count: usize) -> Vec<String> {
    let end = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap();
    (0..count)
        .rev()
        .map(|back| {
            (end - chrono::Duration::weeks(back as i64))
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
    skip: Option<&str>,
) -> Vec<(String, String, f64)> {
    weeks
        .iter()
        .filter(|week| skip != Some(week.as_str()))
        .map(|week| (key.to_string(), week.clone(), quantity))
        .collect()
}

async fn forecast_with(
    workspace: &Workspace,
    revision_id: i32,
    imputation: Option<ImputationMethod>,
) -> String {
    run_forecast(
        workspace,
        revision_id,
        imputation,
        "inbox-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(std::path::PathBuf::from),
    )
    .await
    .unwrap()
    .run_id
}

struct InboxFixture {
    _directory: tempfile::TempDir,
    workspace: Workspace,
    run_a: String,
    run_b: String,
}

/// Two adjacent revisions with two completed runs.
///
/// - `ALPHA`: flat 10 for 40 weeks, then the last 4 weeks jump to 20 (run B).
/// - `GAPPY`: flat 12 with one early gap (imputed in run B), then last 4 weeks 20.
/// - `STEADY`: flat 5 everywhere (no delta).
/// - `SHORT`: only 6 weeks (insufficient history).
async fn inbox_fixture() -> InboxFixture {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let first_weeks = weeks_ending("2026-02-23", 40);
    let second_weeks = weeks_ending("2026-03-23", 44);
    let gap = first_weeks[1].clone();

    let mut rows = Vec::new();
    rows.extend(series_rows("ALPHA", &first_weeks, 10.0, None));
    rows.extend(series_rows("GAPPY", &first_weeks, 12.0, Some(gap.as_str())));
    rows.extend(series_rows("STEADY", &first_weeks, 5.0, None));
    rows.extend(series_rows("SHORT", &first_weeks[34..], 4.0, None));
    let first = write_csv(directory.path(), "first.csv", &canonical(&rows));
    let imported = workspace
        .import_csv(&first, &AtomicBool::new(false))
        .await
        .unwrap();
    let run_a = forecast_with(&workspace, imported.revision_id, None).await;

    let mut rows = Vec::new();
    let mut alpha = series_rows("ALPHA", &second_weeks, 10.0, None);
    for (_, week, quantity) in &mut alpha {
        if second_weeks[40..].contains(week) {
            *quantity = 20.0;
        }
    }
    rows.extend(alpha);
    let mut gappy = series_rows("GAPPY", &second_weeks, 12.0, Some(gap.as_str()));
    for (_, week, quantity) in &mut gappy {
        if second_weeks[40..].contains(week) {
            *quantity = 20.0;
        }
    }
    rows.extend(gappy);
    rows.extend(series_rows("STEADY", &second_weeks, 5.0, None));
    rows.extend(series_rows("SHORT", &second_weeks[38..], 4.0, None));
    let second = write_csv(directory.path(), "second.csv", &canonical(&rows));
    let imported = workspace
        .import_csv(&second, &AtomicBool::new(false))
        .await
        .unwrap();
    let run_b = forecast_with(
        &workspace,
        imported.revision_id,
        Some(ImputationMethod::Last),
    )
    .await;

    InboxFixture {
        _directory: directory,
        workspace,
        run_a,
        run_b,
    }
}

fn find<'a>(
    rows: &'a [opendemand_core::types::InboxItem],
    key: &str,
) -> Option<&'a opendemand_core::types::InboxItem> {
    rows.iter().find(|row| row.series_key == key)
}

#[tokio::test]
async fn inbox_requires_a_completed_run_and_a_valid_filter() {
    let fixture = inbox_fixture().await;
    let error = fixture
        .workspace
        .exception_inbox("0198d3c1-1111-7000-8000-000000000000", "", "all", 0, 20)
        .await
        .unwrap_err();
    assert_eq!(error.code, "unknown_run");
    let error = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "urgent", 0, 20)
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_filter");
    sqlx::query("UPDATE forecast_run SET status = 'failed' WHERE id = ?")
        .bind(
            uuid::Uuid::parse_str(&fixture.run_b)
                .unwrap()
                .as_bytes()
                .as_slice(),
        )
        .execute(&fixture.workspace.pool)
        .await
        .unwrap();
    let error = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 20)
        .await
        .unwrap_err();
    assert_eq!(error.code, "run_not_completed");
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn inbox_lists_rules_by_fixed_priority_without_a_policy() {
    let fixture = inbox_fixture().await;
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    assert!(inbox.material_change_available);
    assert!(inbox.predecessor_run_id.is_some());
    assert_eq!(inbox.policy.version, None, "no policy configured yet");
    // SHORT: insufficient history; GAPPY: imputed input. ALPHA and STEADY
    // have no rule at all while the change policy is disabled.
    assert!(find(&inbox.rows, "ALPHA").is_none());
    assert!(find(&inbox.rows, "STEADY").is_none());
    let short = find(&inbox.rows, "SHORT").expect("SHORT must be listed");
    assert_eq!(short.rules, vec!["insufficient_history".to_string()]);
    assert_eq!(short.outcome, "insufficient_history");
    let gappy = find(&inbox.rows, "GAPPY").expect("GAPPY must be listed");
    assert_eq!(gappy.rules, vec!["imputed_input".to_string()]);
    // Insufficient history (priority 2) sorts before imputed input (priority 3).
    let keys: Vec<&str> = inbox
        .rows
        .iter()
        .map(|row| row.series_key.as_str())
        .collect();
    assert_eq!(keys, vec!["SHORT", "GAPPY"]);
    assert_eq!(inbox.total, 2);
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn material_change_requires_both_thresholds_and_respects_them() {
    let fixture = inbox_fixture().await;
    // Partial thresholds are refused; the rule stays off.
    let error = fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            bias_horizon: None,
            bias_window: None,
            bias_min: None,
            bias_abs: None,
            change_abs: Some("50".into()),
            change_rel: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_policy");
    let error = fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            bias_horizon: None,
            bias_window: None,
            bias_min: None,
            bias_abs: None,
            change_abs: Some("0".into()),
            change_rel: Some("0.5".into()),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_policy");

    let policy = fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            bias_horizon: None,
            bias_window: None,
            bias_min: None,
            bias_abs: None,
            change_abs: Some("50".into()),
            change_rel: Some("0.5".into()),
        })
        .await
        .unwrap();
    assert_eq!(policy.version, Some(1));
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    let alpha = find(&inbox.rows, "ALPHA").expect("ALPHA change must fire");
    assert_eq!(
        alpha.rules,
        vec!["material_change".to_string()],
        "ALPHA has only the change rule"
    );
    let abs = alpha.change_abs.expect("absolute change");
    assert!(
        (abs - 80.0).abs() < 1e-6,
        "8 common weeks x 10 delta: {abs}"
    );
    let rel = alpha.change_rel.expect("relative change");
    assert!((rel - 1.0).abs() < 1e-6, "80/80 = 100%: {rel}");
    // GAPPY has imputed input first, then the change rule.
    let gappy = find(&inbox.rows, "GAPPY").expect("GAPPY must be listed");
    assert_eq!(
        gappy.rules,
        vec!["imputed_input".to_string(), "material_change".to_string()]
    );
    let keys: Vec<&str> = inbox
        .rows
        .iter()
        .map(|row| row.series_key.as_str())
        .collect();
    assert_eq!(keys, vec!["SHORT", "GAPPY", "ALPHA"]);

    // Raising the absolute threshold past the observed change disables the rule
    // again; the policy version is append-only.
    let policy = fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            bias_horizon: None,
            bias_window: None,
            bias_min: None,
            bias_abs: None,
            change_abs: Some("1000".into()),
            change_rel: Some("0.5".into()),
        })
        .await
        .unwrap();
    assert_eq!(policy.version, Some(2));
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    assert!(find(&inbox.rows, "ALPHA").is_none());
    assert_eq!(
        find(&inbox.rows, "GAPPY").unwrap().rules,
        vec!["imputed_input"]
    );

    // The relative threshold alone can veto a firing change.
    fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            bias_horizon: None,
            bias_window: None,
            bias_min: None,
            bias_abs: None,
            change_abs: Some("10".into()),
            change_rel: Some("1.5".into()),
        })
        .await
        .unwrap();
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    assert!(find(&inbox.rows, "ALPHA").is_none());
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn the_first_revision_has_no_material_change_evidence() {
    let fixture = inbox_fixture().await;
    fixture
        .workspace
        .save_review_policy(InboxPolicyEdit {
            bias_horizon: None,
            bias_window: None,
            bias_min: None,
            bias_abs: None,
            change_abs: Some("0.001".into()),
            change_rel: Some("0.001".into()),
        })
        .await
        .unwrap();
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_a, "", "all", 0, 50)
        .await
        .unwrap();
    assert!(!inbox.material_change_available);
    assert!(inbox.predecessor_run_id.is_none());
    assert!(inbox
        .rows
        .iter()
        .all(|row| !row.rules.iter().any(|rule| rule == "material_change")));
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn review_filters_read_derived_state_and_reopen_on_new_evidence() {
    let fixture = inbox_fixture().await;
    let alpha = {
        let inbox = fixture
            .workspace
            .exception_inbox(&fixture.run_b, "", "all", 0, 50)
            .await
            .unwrap();
        let short = find(&inbox.rows, "SHORT").unwrap().clone();
        short
    };
    fixture
        .workspace
        .series_review_set(ReviewActionEdit {
            run_id: fixture.run_b.clone(),
            product_id: alpha.product_id.clone(),
            location_id: alpha.location_id.clone(),
            action: "reviewed".into(),
        })
        .await
        .unwrap();
    let unreviewed = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "unreviewed", 0, 50)
        .await
        .unwrap();
    assert!(find(&unreviewed.rows, "SHORT").is_none());
    let reviewed = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "reviewed", 0, 50)
        .await
        .unwrap();
    let row = find(&reviewed.rows, "SHORT").expect("SHORT stays listed while reviewed");
    assert_eq!(row.review_status, "reviewed");
    assert!(row.reviewed_at.is_some());
    assert_eq!(reviewed.total, 1);

    // A new override revision is new evidence: the acknowledgement reopens.
    fixture
        .workspace
        .append_override(OverrideEdit {
            run_id: fixture.run_b.clone(),
            product_id: alpha.product_id.clone(),
            location_id: alpha.location_id.clone(),
            week_start: "2026-03-30".into(),
            value: Some("4.5".into()),
            expected_revision: 0,
            reason_code: "data_quality".into(),
            note: None,
            review_date: None,
        })
        .await
        .unwrap();
    let unreviewed = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "unreviewed", 0, 50)
        .await
        .unwrap();
    assert!(find(&unreviewed.rows, "SHORT").is_some());
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn due_review_reminders_track_active_overrides_only() {
    let fixture = inbox_fixture().await;
    let baseline = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    // Baseline: no due reminder anywhere (SHORT and GAPPY are listed for other rules).
    assert!(baseline
        .rows
        .iter()
        .all(|row| row.due_review_date.is_none()));

    let series = fixture
        .workspace
        .list_series(2, "STEADY", 0, 10)
        .await
        .unwrap();
    let steady = series.first().unwrap();
    let (product_id, location_id) = (steady.product_id.clone(), steady.location_id.clone());

    // Future review date: no reminder yet.
    fixture
        .workspace
        .append_override(OverrideEdit {
            run_id: fixture.run_b.clone(),
            product_id: product_id.clone(),
            location_id: location_id.clone(),
            week_start: "2026-03-30".into(),
            value: Some("6".into()),
            expected_revision: 0,
            reason_code: "promotion".into(),
            note: None,
            review_date: Some("2999-01-01".into()),
        })
        .await
        .unwrap();
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    assert!(find(&inbox.rows, "STEADY").is_none());

    // A due review date raises the reminder rule while the revision is current.
    fixture
        .workspace
        .append_override(OverrideEdit {
            run_id: fixture.run_b.clone(),
            product_id: product_id.clone(),
            location_id: location_id.clone(),
            week_start: "2026-03-30".into(),
            value: Some("6.5".into()),
            expected_revision: 1,
            reason_code: "promotion".into(),
            note: None,
            review_date: Some("2020-01-01".into()),
        })
        .await
        .unwrap();
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    let steady = find(&inbox.rows, "STEADY").expect("due review must list STEADY");
    assert_eq!(steady.rules, vec!["review_due".to_string()]);
    assert_eq!(steady.due_review_date.as_deref(), Some("2020-01-01"));

    // Clearing the override removes the reminder: it only marks active revisions.
    fixture
        .workspace
        .append_override(OverrideEdit {
            run_id: fixture.run_b.clone(),
            product_id: product_id.clone(),
            location_id: location_id.clone(),
            week_start: "2026-03-30".into(),
            value: None,
            expected_revision: 2,
            reason_code: "customer_information".into(),
            note: None,
            review_date: None,
        })
        .await
        .unwrap();
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    assert!(find(&inbox.rows, "STEADY").is_none());
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn notes_and_search_are_evidence_on_the_inbox_row() {
    let fixture = inbox_fixture().await;
    let series = fixture
        .workspace
        .list_series(2, "SHORT", 0, 10)
        .await
        .unwrap();
    let short = series.first().unwrap();
    fixture
        .workspace
        .series_note_add(SeriesNoteEdit {
            run_id: fixture.run_b.clone(),
            product_id: short.product_id.clone(),
            location_id: short.location_id.clone(),
            note: "Watch this one.".into(),
        })
        .await
        .unwrap();
    let inbox = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "sho", "all", 0, 50)
        .await
        .unwrap();
    assert_eq!(inbox.total, 1);
    let row = find(&inbox.rows, "SHORT").unwrap();
    assert_eq!(row.note_count, 1);
    assert!(row.last_note_at.is_some());

    // Search matches the location code too; pagination keeps the stable order.
    let all = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 50)
        .await
        .unwrap();
    assert_eq!(all.total, 2);
    let first_page = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 0, 1)
        .await
        .unwrap();
    assert_eq!(first_page.total, 2);
    assert_eq!(first_page.rows.len(), 1);
    assert_eq!(first_page.rows[0].series_key, "SHORT");
    let second_page = fixture
        .workspace
        .exception_inbox(&fixture.run_b, "", "all", 1, 1)
        .await
        .unwrap();
    assert_eq!(second_page.rows[0].series_key, "GAPPY");
    fixture.workspace.pool.close().await;
}
