//! Integration tests for the versioned migration path.
//!
//! A workspace written by an older build must not be migrated without a
//! verified pre-migration backup (ADR-0008). These tests build a real version-1
//! database, reopen it with this build, and check the backup pair, the version
//! bump, and the fail-closed behavior when the backup cannot be created.

use opendemand_core::{error::AppError, workspace::Workspace};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{path::Path, sync::atomic::AtomicBool};

/// Open a workspace that must be refused.
async fn open_error(root: std::path::PathBuf) -> AppError {
    match Workspace::open(root).await {
        Ok(_) => panic!("open must fail here"),
        Err(error) => error,
    }
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures")
        .join(name)
}

fn database(root: &Path) -> std::path::PathBuf {
    root.join("workspace.sqlite3")
}

async fn raw_pool(database: &Path) -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(database))
        .await
        .unwrap()
}

/// Reconstruct a version-1 database from a freshly created one: drop the 004,
/// 003, and 002 objects, forget their migration records, and reset the settings
/// version.
async fn downgrade_to_version_one(pool: &sqlx::SqlitePool) {
    sqlx::query("DROP TABLE override_meta")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE series_review")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE series_note")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE forecast_series_status DROP COLUMN imputed_weeks")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE forecast_run DROP COLUMN imputation_method")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE import_config")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE import_revision DROP COLUMN descriptor_json")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version IN (2, 3, 4)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE workspace_settings SET schema_version = '1'")
        .execute(pool)
        .await
        .unwrap();
}

async fn backup_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("backups"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn opening_a_version_one_workspace_backs_up_then_migrates() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let workspace = Workspace::create(root.clone()).await.unwrap();
    assert_eq!(workspace.info().await.unwrap().schema_version, "4");
    workspace
        .import_csv(&fixture("demo.csv"), &AtomicBool::new(false))
        .await
        .unwrap();
    downgrade_to_version_one(&workspace.pool).await;
    workspace.pool.close().await;

    let reopened = Workspace::open(root.clone()).await.unwrap();
    let info = reopened.info().await.unwrap();
    assert_eq!(info.schema_version, "4");
    // The upgrade preserved existing data.
    assert_eq!(info.latest_revision_id, Some(1));
    reopened.pool.close().await;

    // Exactly one verified backup pair, with a manifest that matches the file.
    let names = backup_names(&root).await;
    assert_eq!(
        names.len(),
        2,
        "expected one database and one manifest: {names:?}"
    );
    let backup_file = names
        .iter()
        .find(|name| name.ends_with(".sqlite3"))
        .unwrap()
        .clone();
    let manifest_name = format!("{backup_file}.manifest.json");
    assert!(names.contains(&manifest_name));
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("backups").join(&manifest_name)).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["scope"], "database");
    assert_eq!(manifest["schema_version_before"], "1");
    assert_eq!(manifest["schema_version_after"], "4");
    assert_eq!(manifest["app_version"], env!("CARGO_PKG_VERSION"));
    let bytes = std::fs::read(root.join("backups").join(&backup_file)).unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert_eq!(manifest["sha256"].as_str(), Some(digest.as_str()));
    assert_eq!(manifest["byte_size"].as_u64(), Some(bytes.len() as u64));

    // The backup is a readable database at the old version.
    let backup_pool = raw_pool(&root.join("backups").join(&backup_file)).await;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&backup_pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    let recorded: String =
        sqlx::query_scalar("SELECT schema_version FROM workspace_settings WHERE id = 1")
            .fetch_one(&backup_pool)
            .await
            .unwrap();
    assert_eq!(recorded, "1");
    backup_pool.close().await;
}

#[tokio::test]
async fn migrated_workspace_accepts_saved_import_configurations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let workspace = Workspace::create(root.clone()).await.unwrap();
    downgrade_to_version_one(&workspace.pool).await;
    workspace.pool.close().await;

    let reopened = Workspace::open(root).await.unwrap();
    let profile_id: i64 = sqlx::query_scalar("SELECT id FROM source_profile WHERE is_active = 1")
        .fetch_one(&reopened.pool)
        .await
        .unwrap();
    let timestamp = "2026-10-14T00:00:00Z";
    sqlx::query(
        "INSERT INTO import_config (profile_id, name, descriptor_json, created_at, updated_at)
         VALUES (?, 'Subiekt monthly', '{\"descriptor_version\":1}', ?, ?)",
    )
    .bind(profile_id)
    .bind(timestamp)
    .bind(timestamp)
    .execute(&reopened.pool)
    .await
    .unwrap();

    // One name per profile.
    let duplicate = sqlx::query(
        "INSERT INTO import_config (profile_id, name, descriptor_json, created_at, updated_at)
         VALUES (?, 'Subiekt monthly', '{}', ?, ?)",
    )
    .bind(profile_id)
    .bind(timestamp)
    .bind(timestamp)
    .execute(&reopened.pool)
    .await;
    assert!(
        duplicate.is_err(),
        "a duplicate configuration name must fail"
    );

    // A preprocessed revision can carry its frozen descriptor next to a
    // canonical one that stays NULL.
    sqlx::query(
        "INSERT INTO import_revision (profile_id, content_hash, mapping_fingerprint, contract_version,
             source_filename, imported_at, row_count, observation_count, issue_count, descriptor_json)
         VALUES (?, 'sha256:raw', 'sha256:rawmap', 'preprocess/1', 'subiekt.csv', ?, 1, 1, 0, '{\"descriptor_version\":1}')",
    )
    .bind(profile_id)
    .bind(timestamp)
    .execute(&reopened.pool)
    .await
    .unwrap();
    let descriptors: Vec<Option<String>> =
        sqlx::query_scalar("SELECT descriptor_json FROM import_revision ORDER BY id")
            .fetch_all(&reopened.pool)
            .await
            .unwrap();
    assert_eq!(descriptors.len(), 1);
    assert!(descriptors[0].is_some());
    reopened.pool.close().await;
}

#[tokio::test]
async fn failed_backup_leaves_the_workspace_at_version_one() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let workspace = Workspace::create(root.clone()).await.unwrap();
    downgrade_to_version_one(&workspace.pool).await;
    workspace.pool.close().await;

    // Block the backup directory: a plain file where the directory must go.
    std::fs::remove_dir_all(root.join("backups")).unwrap();
    std::fs::write(root.join("backups"), b"blocked").unwrap();

    let error = open_error(root.clone()).await;
    assert_eq!(error.code, "backup_failed");

    // Nothing was migrated.
    let pool = raw_pool(&database(&root)).await;
    let version: String =
        sqlx::query_scalar("SELECT schema_version FROM workspace_settings WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(version, "1");
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'import_config'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tables, 0);
    pool.close().await;
}

#[tokio::test]
async fn a_newer_schema_version_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    let workspace = Workspace::create(root.clone()).await.unwrap();
    sqlx::query("UPDATE workspace_settings SET schema_version = '5'")
        .execute(&workspace.pool)
        .await
        .unwrap();
    workspace.pool.close().await;

    let error = open_error(root).await;
    assert_eq!(error.code, "unsupported_schema");
}
