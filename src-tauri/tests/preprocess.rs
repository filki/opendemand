//! Integration tests for the preprocessing import path.
//!
//! These exercise `Workspace::import_raw_csv` against real temporary workspaces:
//! detection fixtures in three source shapes, weekly aggregation, descriptor
//! idempotency, retained raw bytes, capped issues, and blocking failures.

use opendemand_core::{
    preprocess::{preview, ParsingDescriptor},
    types::ImportResult,
    workspace::Workspace,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/architecture/contracts/examples/import")
        .join(name)
}

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

fn inline_descriptor() -> ParsingDescriptor {
    ParsingDescriptor {
        format: Some("csv".into()),
        encoding: Some("utf-8".into()),
        delimiter: Some(";".into()),
        header_row: Some(1),
        key_columns: Some(vec!["Kod".into()]),
        date_column: Some("Data".into()),
        date_formats: Some(vec!["%d.%m.%Y".into()]),
        quantity_column: Some("Ilosc".into()),
        ..ParsingDescriptor::default()
    }
}

fn xlsx_descriptor() -> ParsingDescriptor {
    ParsingDescriptor {
        format: Some("xlsx".into()),
        sheet: Some("Sprzedaz".into()),
        header_row: Some(3),
        key_columns: Some(vec!["Kod".into()]),
        date_column: Some("Data".into()),
        date_formats: Some(vec!["%Y-%m-%d".into()]),
        quantity_column: Some("Sprzedaz".into()),
        ..ParsingDescriptor::default()
    }
}

async fn import_raw(
    workspace: &Workspace,
    path: &Path,
    descriptor: &ParsingDescriptor,
) -> ImportResult {
    workspace
        .import_raw(path, descriptor, &AtomicBool::new(false))
        .await
        .unwrap()
}

async fn observation(
    workspace: &Workspace,
    revision_id: i32,
    sku: &str,
    week: &str,
) -> Option<f64> {
    sqlx::query_scalar(
        "SELECT o.quantity FROM observation o
         JOIN product p ON p.id = o.product_id
         WHERE o.revision_id = ? AND p.sku = ? AND o.week_start = ?",
    )
    .bind(revision_id)
    .bind(sku)
    .bind(week)
    .fetch_optional(&workspace.pool)
    .await
    .unwrap()
    .flatten()
}

#[tokio::test]
async fn windows1250_subiekt_fixture_produces_weekly_sums() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let source = example("subiekt-sprzedaz.csv");
    let imported = import_raw(&workspace, &source, &subiekt_descriptor()).await;

    assert_eq!(imported.revision_id, 1);
    assert!(!imported.is_duplicate);
    assert_eq!(imported.series_count, 8);
    assert_eq!(imported.observation_count, 24);
    assert_eq!(imported.last_week, "2011-12-05");
    // Six cells contain identical duplicate rows; each is reported once.
    assert_eq!(imported.issue_count, 6);

    assert_eq!(
        observation(&workspace, 1, "15039", "2011-11-21").await,
        Some(93.0)
    );
    assert_eq!(
        observation(&workspace, 1, "15039", "2011-11-28").await,
        Some(2.0)
    );
    assert_eq!(
        observation(&workspace, 1, "15039", "2011-12-05").await,
        Some(46.0)
    );
    assert_eq!(
        observation(&workspace, 1, "23020", "2011-12-05").await,
        Some(0.0)
    );

    let (quantity, status): (f64, String) = sqlx::query_as(
        "SELECT o.quantity, o.status FROM observation o
         JOIN product p ON p.id = o.product_id
         WHERE o.revision_id = 1 AND p.sku = '16219' AND o.week_start = '2011-12-05'",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(quantity, -42.0);
    assert_eq!(status, "return");

    let row_count: i64 = sqlx::query_scalar("SELECT row_count FROM import_revision WHERE id = 1")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(row_count, 101);
    let contract: String =
        sqlx::query_scalar("SELECT contract_version FROM import_revision WHERE id = 1")
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(contract, "preprocess/1");

    // The retained copy is the untouched original file.
    let retained = std::fs::read(workspace.path.join("sources/1/source.csv")).unwrap();
    assert_eq!(retained, std::fs::read(&source).unwrap());
    workspace.pool.close().await;
}

#[tokio::test]
async fn same_bytes_same_descriptor_is_noop() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let descriptor = subiekt_descriptor();
    let first = import_raw(&workspace, &example("subiekt-sprzedaz.csv"), &descriptor).await;
    let second = import_raw(&workspace, &example("subiekt-sprzedaz.csv"), &descriptor).await;

    assert!(second.is_duplicate);
    assert_eq!(second.revision_id, first.revision_id);
    assert_eq!(second.observation_count, first.observation_count);
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 1);
    workspace.pool.close().await;
}

#[tokio::test]
async fn descriptor_change_creates_new_revision() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let source = example("subiekt-sprzedaz.csv");
    let first = import_raw(&workspace, &source, &subiekt_descriptor()).await;
    assert_eq!(first.revision_id, 1);

    // Same bytes, different quantity binding: a new immutable revision.
    let mut changed = subiekt_descriptor();
    changed.quantity_column = Some("Wartość netto".into());
    changed.decimal_separator = Some(",".into());
    let second = import_raw(&workspace, &source, &changed).await;
    assert!(!second.is_duplicate);
    assert_eq!(second.revision_id, 2);
    assert_eq!(second.observation_count, 24);

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
async fn canonical_bytes_through_raw_path_create_a_separate_revision() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let source = fixture("demo.csv");
    let canonical = workspace
        .import_csv(&source, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(canonical.revision_id, 1);

    let descriptor = ParsingDescriptor {
        format: Some("csv".into()),
        encoding: Some("utf-8".into()),
        delimiter: Some(",".into()),
        header_row: Some(1),
        key_columns: Some(vec!["series_key".into()]),
        date_column: Some("week_start".into()),
        date_formats: Some(vec!["%Y-%m-%d".into()]),
        quantity_column: Some("quantity".into()),
        ..ParsingDescriptor::default()
    };
    let raw = import_raw(&workspace, &source, &descriptor).await;
    assert!(!raw.is_duplicate);
    assert_eq!(raw.revision_id, 2);
    assert_eq!(raw.observation_count, canonical.observation_count);

    let contract: String =
        sqlx::query_scalar("SELECT contract_version FROM import_revision WHERE id = 2")
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(contract, "preprocess/1");
    workspace.pool.close().await;
}

#[tokio::test]
async fn empty_key_footer_rows_are_excluded_with_warning() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let path = write_csv(
        directory.path(),
        "footer.csv",
        "Kod;Data;Ilosc\n15039;21.11.2011;10\n;30.11.2011;99\n",
    );
    let imported = import_raw(&workspace, &path, &inline_descriptor()).await;

    assert_eq!(imported.observation_count, 1);
    assert_eq!(imported.series_count, 1);
    assert_eq!(imported.issue_count, 1);
    let warning: String = sqlx::query_scalar(
        "SELECT code FROM import_issue WHERE revision_id = 1 AND code = 'empty_key_row'",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(warning, "empty_key_row");
    assert_eq!(
        observation(&workspace, 1, "15039", "2011-11-21").await,
        Some(10.0)
    );
    workspace.pool.close().await;
}

#[tokio::test]
async fn key_separator_inside_a_part_is_excluded() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let path = write_csv(
        directory.path(),
        "separator.csv",
        "Kod;Werk;Data;Ilosc\nA|B;1000;21.11.2011;5\nA;B;21.11.2011;5\n",
    );
    let mut descriptor = inline_descriptor();
    descriptor.key_columns = Some(vec!["Kod".into(), "Werk".into()]);
    let imported = import_raw(&workspace, &path, &descriptor).await;

    assert_eq!(imported.series_count, 1);
    assert_eq!(imported.observation_count, 1);
    assert_eq!(
        observation(&workspace, 1, "A|B", "2011-11-21").await,
        Some(5.0)
    );
    let codes: Vec<String> =
        sqlx::query_scalar("SELECT code FROM import_issue WHERE revision_id = 1")
            .fetch_all(&workspace.pool)
            .await
            .unwrap();
    assert_eq!(codes, vec!["key_separator_in_part".to_string()]);
    workspace.pool.close().await;
}

#[tokio::test]
async fn decimal_and_thousands_separators_parse() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let path = write_csv(
        directory.path(),
        "money.csv",
        "Kod;Data;Ilosc\n15039;21.11.2011;1.234,5\n",
    );
    let mut descriptor = inline_descriptor();
    descriptor.decimal_separator = Some(",".into());
    descriptor.thousands_separator = Some(".".into());
    let imported = import_raw(&workspace, &path, &descriptor).await;

    assert_eq!(imported.observation_count, 1);
    assert_eq!(
        observation(&workspace, 1, "15039", "2011-11-21").await,
        Some(1234.5)
    );
    workspace.pool.close().await;
}

#[tokio::test]
async fn date_variants_bucket_to_monday() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let path = write_csv(
        directory.path(),
        "dates.csv",
        "Kod;Data;Ilosc\nA;2021-11-24;1\nA;20211128;2\nA;22.11.2021;3\n",
    );
    let mut descriptor = inline_descriptor();
    descriptor.date_formats = Some(vec!["%Y-%m-%d".into(), "%Y%m%d".into(), "%d.%m.%Y".into()]);
    let imported = import_raw(&workspace, &path, &descriptor).await;

    // Wednesday, Sunday, and Monday all land in the week of Monday 2021-11-22.
    assert_eq!(imported.observation_count, 1);
    assert_eq!(
        observation(&workspace, 1, "A", "2021-11-22").await,
        Some(6.0)
    );
    workspace.pool.close().await;
}

#[tokio::test]
async fn unparsable_rows_are_excluded_with_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let path = write_csv(
        directory.path(),
        "broken-rows.csv",
        "Kod;Data;Ilosc\nA;not-a-date;1\nA;21.11.2011;abc\nA;21.11.2011;5\n",
    );
    let imported = import_raw(&workspace, &path, &inline_descriptor()).await;

    assert_eq!(imported.observation_count, 1);
    assert_eq!(imported.issue_count, 2);
    assert_eq!(
        observation(&workspace, 1, "A", "2011-11-21").await,
        Some(5.0)
    );
    workspace.pool.close().await;
}

#[tokio::test]
async fn issues_are_capped_per_code_with_a_summary() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let mut csv = String::from("Kod;Data;Ilosc\n");
    for index in 0..60 {
        csv.push_str(&format!("A;not-a-date-{index};1\n"));
    }
    csv.push_str("A;21.11.2011;5\n");
    let path = write_csv(directory.path(), "many-issues.csv", &csv);
    let imported = import_raw(&workspace, &path, &inline_descriptor()).await;

    // 50 stored rows plus one summary for the truncated code.
    assert_eq!(imported.issue_count, 51);
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM import_issue WHERE revision_id = 1 AND code = 'invalid_date_row'",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(stored, 51);
    let summaries: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM import_issue WHERE revision_id = 1 AND code = 'invalid_date_row' AND row_ref IS NULL",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(summaries, 1);
    assert_eq!(
        observation(&workspace, 1, "A", "2011-11-21").await,
        Some(5.0)
    );
    workspace.pool.close().await;
}

#[tokio::test]
async fn missing_bound_column_fails_before_commit() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let mut descriptor = subiekt_descriptor();
    descriptor.quantity_column = Some("Nie ma".into());
    let error = workspace
        .import_raw(
            &example("subiekt-sprzedaz.csv"),
            &descriptor,
            &AtomicBool::new(false),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "missing_quantity_column");
    assert_eq!(workspace.info().await.unwrap().latest_revision_id, None);
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 0);
    workspace.pool.close().await;
}

#[tokio::test]
async fn unsupported_format_fails_before_commit() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let path = write_csv(directory.path(), "legacy.xls", "old format\n");
    let error = workspace
        .import_raw(&path, &inline_descriptor(), &AtomicBool::new(false))
        .await
        .unwrap_err();

    assert_eq!(error.code, "unsupported_format");
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 0);
    workspace.pool.close().await;
}

#[tokio::test]
async fn xlsx_and_csv_fixtures_produce_identical_observations() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let csv = import_raw(
        &workspace,
        &example("subiekt-sprzedaz.csv"),
        &subiekt_descriptor(),
    )
    .await;
    let xlsx = import_raw(
        &workspace,
        &example("excel-raport-sprzedazy.xlsx"),
        &xlsx_descriptor(),
    )
    .await;

    assert_eq!(xlsx.revision_id, 2);
    assert_eq!(xlsx.series_count, csv.series_count);
    assert_eq!(xlsx.observation_count, csv.observation_count);
    assert_eq!(xlsx.last_week, "2011-12-05");
    assert_eq!(xlsx.issue_count, 6);
    assert_eq!(
        observation(&workspace, 2, "15039", "2011-11-21").await,
        Some(93.0)
    );

    // Every CSV cell has an identical workbook cell, and the counts are equal.
    let matching: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM observation a
         JOIN observation b ON b.revision_id = 2 AND b.product_id = a.product_id
                           AND b.week_start = a.week_start AND b.quantity = a.quantity
         WHERE a.revision_id = 1",
    )
    .fetch_one(&workspace.pool)
    .await
    .unwrap();
    assert_eq!(matching, csv.observation_count as i64);
    workspace.pool.close().await;
}

#[tokio::test]
async fn xlsx_preview_lists_sheets_and_finds_header_row() {
    let descriptor = xlsx_descriptor();
    let preview = preview(&example("excel-raport-sprzedazy.xlsx"), Some(&descriptor)).unwrap();

    assert_eq!(preview.sheets, vec!["Sprzedaz".to_string()]);
    assert_eq!(preview.detected.sheet.as_deref(), Some("Sprzedaz"));
    assert_eq!(preview.detected.header_row, Some(3));
    assert_eq!(
        preview.headers,
        vec![
            "Data",
            "Kod",
            "Nazwa towaru",
            "Sprzedaz",
            "Cena jedn.",
            "Waluta"
        ]
    );
    assert_eq!(preview.sample_rows.len(), 50);
    assert!(preview.can_parse);
    let first = &preview.parsed_rows[0];
    assert_eq!(first.series_key, "15039");
    assert_eq!(first.week_start, "2011-11-21");
    assert_eq!(first.quantity, 10.0);
    assert_eq!(first.status, "known");
}

#[tokio::test]
async fn xlsx_sheet_not_found_is_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let mut descriptor = xlsx_descriptor();
    descriptor.sheet = Some("Nie ma".into());
    let error = workspace
        .import_raw(
            &example("excel-raport-sprzedazy.xlsx"),
            &descriptor,
            &AtomicBool::new(false),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "sheet_not_found");

    // The preview reports the same code and still lists the available sheets.
    let preview = preview(&example("excel-raport-sprzedazy.xlsx"), Some(&descriptor)).unwrap();
    assert!(preview
        .findings
        .iter()
        .any(|finding| finding.severity == "blocking" && finding.code == "sheet_not_found"));
    assert_eq!(preview.sheets, vec!["Sprzedaz".to_string()]);

    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_revision")
        .fetch_one(&workspace.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 0);
    workspace.pool.close().await;
}

#[tokio::test]
async fn import_config_round_trips_and_updates_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let descriptor = subiekt_descriptor();
    let saved = workspace
        .save_import_config("  Subiekt monthly  ", &descriptor)
        .await
        .unwrap();
    assert_eq!(saved.name, "Subiekt monthly");
    let listed = workspace.list_import_configs().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].descriptor, descriptor);

    // Saving the same name updates the row instead of adding another one.
    let mut changed = descriptor.clone();
    changed.delimiter = Some(",".into());
    let updated = workspace
        .save_import_config("Subiekt monthly", &changed)
        .await
        .unwrap();
    assert_eq!(updated.id, saved.id);
    let listed = workspace.list_import_configs().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].descriptor.delimiter.as_deref(), Some(","));

    workspace.delete_import_config(saved.id).await.unwrap();
    assert!(workspace.list_import_configs().await.unwrap().is_empty());
    workspace.pool.close().await;
}

#[tokio::test]
async fn blank_config_names_and_unknown_deletes_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let blank = workspace
        .save_import_config("   ", &subiekt_descriptor())
        .await
        .unwrap_err();
    assert_eq!(blank.code, "invalid_config");
    let missing = workspace.delete_import_config(42).await.unwrap_err();
    assert_eq!(missing.code, "config_not_found");
    workspace.pool.close().await;
}

#[tokio::test]
async fn raw_import_freezes_the_descriptor_and_canonical_stays_null() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let descriptor = subiekt_descriptor();
    let raw = import_raw(&workspace, &example("subiekt-sprzedaz.csv"), &descriptor).await;
    let canonical = workspace
        .import_csv(&fixture("demo.csv"), &AtomicBool::new(false))
        .await
        .unwrap();

    let frozen: String =
        sqlx::query_scalar("SELECT descriptor_json FROM import_revision WHERE id = ?")
            .bind(raw.revision_id)
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    let frozen: ParsingDescriptor = serde_json::from_str(&frozen).unwrap();
    assert_eq!(frozen, descriptor);
    let direct: Option<String> =
        sqlx::query_scalar("SELECT descriptor_json FROM import_revision WHERE id = ?")
            .bind(canonical.revision_id)
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    assert!(direct.is_none());
    workspace.pool.close().await;
}

#[tokio::test]
async fn editing_a_config_never_rewrites_history() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let source = example("subiekt-sprzedaz.csv");
    let first = subiekt_descriptor();
    let saved = workspace
        .save_import_config("Subiekt monthly", &first)
        .await
        .unwrap();
    let revision_one = import_raw(&workspace, &source, &first).await;

    // Edit the saved configuration, then import the same bytes with it.
    let mut second = first.clone();
    second.quantity_column = Some("Wartość netto".into());
    second.decimal_separator = Some(",".into());
    workspace
        .save_import_config(&saved.name, &second)
        .await
        .unwrap();
    let revision_two = import_raw(&workspace, &source, &second).await;
    assert_ne!(revision_two.revision_id, revision_one.revision_id);

    // Revision one keeps the descriptor it was committed with.
    let frozen: String =
        sqlx::query_scalar("SELECT descriptor_json FROM import_revision WHERE id = ?")
            .bind(revision_one.revision_id)
            .fetch_one(&workspace.pool)
            .await
            .unwrap();
    let frozen: ParsingDescriptor = serde_json::from_str(&frozen).unwrap();
    assert_eq!(frozen, first);

    // Re-importing with the frozen descriptor is still a no-op.
    let again = import_raw(&workspace, &source, &first).await;
    assert!(again.is_duplicate);
    assert_eq!(again.revision_id, revision_one.revision_id);
    workspace.pool.close().await;
}
