//! Integration tests for the opt-in sparse model set (ADR-0032): Croston-SBA
//! and TSB are planned only on complete, non-negative training windows, the
//! default set is unchanged, and the frozen configuration records the set.

use opendemand_core::{
    forecast::{run_forecast, run_forecast_with_model_set},
    imputation::ImputationMethod,
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

struct SeriesPlan {
    key: &'static str,
    flat: f64,
    zero: bool,
    /// Zero-based week positions that get the flat value even when zeroed.
    spikes: Vec<usize>,
    /// Week position (from the end) that becomes a negative return.
    negative_at: Option<usize>,
    /// Week position (from the end) that is omitted, creating a gap.
    gap_at: Option<usize>,
}

fn rows_for(weeks: &[String], plan: &SeriesPlan) -> Vec<(String, String, f64)> {
    let length = weeks.len();
    let mut rows = Vec::new();
    for (index, week) in weeks.iter().enumerate() {
        if let Some(gap) = plan.gap_at {
            if index == length - gap {
                continue;
            }
        }
        let mut value = if plan.zero { 0.0 } else { plan.flat };
        if plan.spikes.contains(&index) {
            value = plan.flat;
        }
        if let Some(negative) = plan.negative_at {
            if index == length - negative {
                value = -3.0;
            }
        }
        rows.push((plan.key.to_string(), week.clone(), value));
    }
    rows
}

fn canonical(rows: &[(String, String, f64)]) -> String {
    let mut content = String::from("series_key,week_start,quantity\n");
    for (key, week, quantity) in rows {
        content.push_str(&format!("{key},{week},{quantity}\n"));
    }
    content
}

async fn sparse_workspace() -> (tempfile::TempDir, Workspace, i32) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let weeks = weeks_ending("2026-02-23", 60);
    let plans = [
        SeriesPlan {
            key: "SMOOTH",
            flat: 10.0,
            zero: false,
            spikes: vec![],
            negative_at: None,
            gap_at: None,
        },
        SeriesPlan {
            key: "INTER",
            flat: 5.0,
            zero: true,
            spikes: vec![10, 25, 40, 55],
            negative_at: None,
            gap_at: None,
        },
        SeriesPlan {
            key: "NEG",
            flat: 8.0,
            zero: false,
            spikes: vec![],
            negative_at: Some(5),
            gap_at: None,
        },
        SeriesPlan {
            key: "GAPPY",
            flat: 12.0,
            zero: false,
            spikes: vec![],
            negative_at: None,
            gap_at: Some(13),
        },
    ];
    let mut rows = Vec::new();
    for plan in &plans {
        rows.extend(rows_for(&weeks, plan));
    }
    let source = write_csv(directory.path(), "sales.csv", &canonical(&rows));
    let imported = workspace
        .import_csv(&source, &AtomicBool::new(false))
        .await
        .unwrap();
    (directory, workspace, imported.revision_id)
}

async fn run_with(
    workspace: &Workspace,
    revision_id: i32,
    model_set: &str,
    imputation: Option<ImputationMethod>,
) -> String {
    run_forecast_with_model_set(
        workspace,
        revision_id,
        imputation,
        model_set,
        "sparse-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(std::path::PathBuf::from),
    )
    .await
    .unwrap()
    .run_id
}

#[tokio::test]
async fn sparse_set_plans_intermittent_candidates_and_freezes_the_config() {
    let (_directory, workspace, revision_id) = sparse_workspace().await;
    let run = run_with(&workspace, revision_id, "sparse", None).await;

    let models: Vec<(String, String)> =
        sqlx::query_as("SELECT model_id, status FROM run_model WHERE run_id = ? ORDER BY model_id")
            .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
            .fetch_all(&workspace.pool)
            .await
            .unwrap();
    let ids: Vec<&str> = models.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "auto_ets_52",
            "croston_sba",
            "naive",
            "seasonal_naive_52",
            "tsb"
        ]
    );
    for (id, status) in &models {
        // AutoETS keeps its 104-week floor: on a 60-week history it plans
        // nothing; every other model must complete.
        if id == "auto_ets_52" || id == "seasonal_naive_52" {
            assert_eq!(status, "not_run", "{id} stays below its floor");
        } else {
            assert_eq!(status, "completed", "{id} must complete");
        }
    }

    let config: String = sqlx::query_scalar("SELECT config_json FROM forecast_run WHERE id = ?")
        .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert!(config.contains("\"model_set\":\"sparse\""), "{config}");
    assert!(config.contains("croston_sba"), "{config}");
    assert!(config.contains("tsb"), "{config}");

    // INTER is the intermittent series: the sparse candidates are evaluated on
    // the same criteria as everything else, with champion eligibility rules.
    let inter_sparse: Vec<(String, String)> = sqlx::query_as(
        "SELECT e.model_id, e.eligibility_status FROM series_model_evaluation e
         JOIN product p ON p.id = e.product_id
         WHERE e.run_id = ? AND p.sku = 'INTER' AND e.model_id IN ('croston_sba', 'tsb')
         ORDER BY e.model_id",
    )
    .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
    .fetch_all(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(inter_sparse.len(), 2);
    for (model, eligibility) in &inter_sparse {
        assert_eq!(eligibility, "champion_eligible", "{model} on INTER");
    }

    // NEG carries a negative return inside its last 12 weeks: the sparse
    // candidates are never planned and say so instead of guessing.
    let neg: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT e.model_id, e.eligibility_status, e.production_status
         FROM series_model_evaluation e JOIN product p ON p.id = e.product_id
         WHERE e.run_id = ? AND p.sku = 'NEG' AND e.model_id IN ('croston_sba', 'tsb')
         ORDER BY e.model_id",
    )
    .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
    .fetch_all(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(neg.len(), 2);
    for (model, eligibility, production) in &neg {
        assert_eq!(eligibility, "not_eligible", "{model} on NEG");
        assert_eq!(production, "not_run", "{model} on NEG");
    }
    workspace.pool.close().await;
}

#[tokio::test]
async fn imputed_input_disqualifies_sparse_candidates_with_a_disclosure() {
    let (_directory, workspace, revision_id) = sparse_workspace().await;
    // GAPPY carries a filled week: its effective window contains an imputed
    // training point, so the sparse candidates are disclosed as not eligible
    // instead of being planned on invented values.
    let run = run_with(
        &workspace,
        revision_id,
        "sparse",
        Some(ImputationMethod::Last),
    )
    .await;
    let gappy: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT e.model_id, e.eligibility_status, e.production_status
         FROM series_model_evaluation e JOIN product p ON p.id = e.product_id
         WHERE e.run_id = ? AND p.sku = 'GAPPY' AND e.model_id IN ('croston_sba', 'tsb')
         ORDER BY e.model_id",
    )
    .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
    .fetch_all(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(gappy.len(), 2);
    for (model, eligibility, production) in &gappy {
        assert_eq!(eligibility, "not_eligible", "{model}: {gappy:?}");
        assert_eq!(production, "not_run", "{model}: {gappy:?}");
    }
    // Clean series stay eligible under the same imputing run.
    let smooth: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM series_model_evaluation e JOIN product p ON p.id = e.product_id
         WHERE e.run_id = ? AND p.sku = 'SMOOTH' AND e.model_id IN ('croston_sba', 'tsb')
           AND e.eligibility_status = 'champion_eligible'",
    )
    .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(smooth, 2);
    let gappy_imputed: i64 =
        sqlx::query_scalar("SELECT imputed_weeks FROM forecast_series_status s JOIN product p ON p.id = s.product_id WHERE s.run_id = ? AND p.sku = 'GAPPY'")
            .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(gappy_imputed, 1);
    workspace.pool.close().await;
}

#[tokio::test]
async fn the_default_set_never_plans_sparse_models() {
    let (_directory, workspace, revision_id) = sparse_workspace().await;
    let run = run_forecast(
        &workspace,
        revision_id,
        None,
        "sparse-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(std::path::PathBuf::from),
    )
    .await
    .unwrap()
    .run_id;
    let sparse_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM series_model_evaluation WHERE run_id = ? AND model_id IN ('croston_sba', 'tsb')",
    )
    .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(sparse_rows, 0);
    let config: String = sqlx::query_scalar("SELECT config_json FROM forecast_run WHERE id = ?")
        .bind(uuid::Uuid::parse_str(&run).unwrap().as_bytes().as_slice())
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert!(config.contains("\"model_set\":\"default\""));
    assert!(!config.contains("croston_sba"));
    // An unknown set id is refused, never silently defaulted.
    let error = run_forecast_with_model_set(
        &workspace,
        revision_id,
        None,
        "turbo",
        "sparse-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(std::path::PathBuf::from),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_model_set");
    workspace.pool.close().await;
}
