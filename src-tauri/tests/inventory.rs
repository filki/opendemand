//! Integration tests for the inventory/1 batch and the derived replenishment
//! plan (ADR-0031). The week timeline is built relative to the current week so
//! the fixture stays valid on any run date.

use chrono::{Datelike, Duration, NaiveDate, Utc};
use opendemand_core::{
    forecast::run_forecast,
    types::{InventoryImportEdit, PublicationEdit, ReplenishmentEdit},
    workspace::Workspace,
};
use std::{
    io::BufReader,
    path::{Path, PathBuf},
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

fn series_rows(key: &str, weeks: &[String], quantity: f64) -> Vec<(String, String, f64)> {
    weeks
        .iter()
        .map(|week| (key.to_string(), week.clone(), quantity))
        .collect()
}

fn write_file(directory: &Path, name: &str, content: &str) -> String {
    let path = directory.join(name);
    std::fs::write(&path, content).unwrap();
    path.display().to_string()
}

struct InventoryFixture {
    _directory: tempfile::TempDir,
    workspace: Workspace,
    publication_id: i32,
    horizon_start: String,
    files: PathBuf,
}

/// Sales history through the last finished week, one run, and a real
/// publication pinned to it.
async fn inventory_fixture() -> InventoryFixture {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::create(directory.path().join("workspace"))
        .await
        .unwrap();
    let today = Utc::now().date_naive();
    let current_monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let cutoff = current_monday - Duration::weeks(1);
    let horizon_start = current_monday;
    let weeks = weeks_ending_at(cutoff, 40);
    let mut rows = Vec::new();
    rows.extend(series_rows("STOCK1", &weeks, 10.0));
    rows.extend(series_rows("STOCK2", &weeks, 5.0));
    rows.extend(series_rows("SHORT", &weeks[34..], 4.0));
    let source = directory.path().join("sales.csv");
    std::fs::write(&source, canonical(&rows)).unwrap();
    let imported = workspace
        .import_csv(&source, &AtomicBool::new(false))
        .await
        .unwrap();
    let run = run_forecast(
        &workspace,
        imported.revision_id,
        None,
        "inventory-test",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
    .await
    .unwrap();
    let publication = workspace
        .publish_plan(PublicationEdit { run_id: run.run_id })
        .await
        .unwrap();
    let files = directory.path().join("inventory");
    std::fs::create_dir_all(&files).unwrap();
    InventoryFixture {
        _directory: directory,
        workspace,
        publication_id: publication.id,
        horizon_start: horizon_start.format("%Y-%m-%d").to_string(),
        files,
    }
}

fn csv_edit(
    fixture: &InventoryFixture,
    stock: &str,
    receipts: &str,
    policy: &str,
) -> InventoryImportEdit {
    // An empty receipts table is expressed as "no file" plus the confirmation.
    let (receipts_path, receipts_empty_confirmed) = if receipts.is_empty() {
        (None, true)
    } else {
        (
            Some(write_file(&fixture.files, "receipts.csv", receipts)),
            false,
        )
    };
    InventoryImportEdit {
        stock_path: Some(write_file(&fixture.files, "stock.csv", stock)),
        receipts_path,
        policy_path: Some(write_file(&fixture.files, "policy.csv", policy)),
        workbook_path: None,
        receipts_empty_confirmed,
        unit_confirmed: true,
    }
}

fn standard_edit(fixture: &InventoryFixture) -> InventoryImportEdit {
    let as_of = &fixture.horizon_start;
    let receipt_week = (NaiveDate::parse_from_str(as_of, "%Y-%m-%d").unwrap() + Duration::weeks(1))
        .format("%Y-%m-%d")
        .to_string();
    let stock = format!("sku,stock,as_of\nSTOCK1,3,{as_of}\nSTOCK2,100,{as_of}\n");
    let receipts = format!("receipt_id,sku,quantity,available_week\nR1,STOCK1,5,{receipt_week}\n");
    let policy = "sku,unit,lead_weeks,review_weeks,safety_stock,moq,pack_size\n\
                  STOCK1,EA,2,1,4,10,6\n\
                  STOCK2,EA,1,1,0,0,1\n";
    csv_edit(fixture, &stock, &receipts, policy)
}

#[tokio::test]
async fn inventory_preview_validates_exactly_and_blocks_explicitly() {
    let fixture = inventory_fixture().await;
    let preview = fixture
        .workspace
        .inventory_preview(standard_edit(&fixture))
        .await
        .unwrap();
    assert!(preview.can_import);
    assert_eq!(preview.sku_count, 2);
    assert_eq!(preview.receipt_count, 1);
    assert_eq!(
        preview.as_of_week.as_deref(),
        Some(fixture.horizon_start.as_str())
    );

    // Unknown SKU, missing policy, missing stock, and unconfirmed units block.
    let as_of = &fixture.horizon_start;
    let stock = format!("sku,stock,as_of\nNOPE,1,{as_of}\nMISSINGSTOCK,1,{as_of}\n");
    let policy = "sku,unit,lead_weeks,review_weeks,safety_stock,moq,pack_size\n\
                  NOPE,EA,1,1,0,0,1\n\
                  MISSINGPOLICY,EA,1,1,0,0,1\n";
    let mut edit = csv_edit(&fixture, &stock, "", policy);
    edit.unit_confirmed = false;
    edit.receipts_empty_confirmed = false;
    let preview = fixture.workspace.inventory_preview(edit).await.unwrap();
    assert!(!preview.can_import);
    let codes: Vec<&str> = preview
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();
    assert!(codes.contains(&"inventory_unknown_sku"), "{codes:?}");
    assert!(codes.contains(&"inventory_missing_policy"), "{codes:?}");
    assert!(codes.contains(&"inventory_missing_stock"), "{codes:?}");
    assert!(codes.contains(&"inventory_unit_confirmation"), "{codes:?}");
    assert!(
        codes.contains(&"inventory_receipts_unconfirmed"),
        "{codes:?}"
    );

    // A Tuesday as-of date, a bad pack size, and duplicate receipt keys block.
    let tuesday = (NaiveDate::parse_from_str(as_of, "%Y-%m-%d").unwrap() + Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let stock = format!("sku,stock,as_of\nSTOCK1,3,{tuesday}\n");
    let receipts = format!(
        "receipt_id,sku,quantity,available_week\nR1,STOCK1,1,{as_of}\nR1,STOCK1,2,{as_of}\n"
    );
    let policy =
        "sku,unit,lead_weeks,review_weeks,safety_stock,moq,pack_size\nSTOCK1,EA,1,1,0,0,0\n";
    let preview = fixture
        .workspace
        .inventory_preview(csv_edit(&fixture, &stock, &receipts, policy))
        .await
        .unwrap();
    assert!(!preview.can_import);
    let codes: Vec<&str> = preview
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();
    assert!(codes.contains(&"inventory_invalid_row"), "{codes:?}");
    assert!(codes.contains(&"inventory_duplicate"), "{codes:?}");
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn inventory_import_is_immutable_retained_and_idempotent() {
    let fixture = inventory_fixture().await;
    let edit = standard_edit(&fixture);
    let snapshot = fixture
        .workspace
        .inventory_import(edit.clone())
        .await
        .unwrap();
    assert_eq!(snapshot.as_of_week, fixture.horizon_start);
    assert_eq!(snapshot.sku_count, 2);
    assert_eq!(snapshot.receipt_count, 1);
    assert!(!snapshot.is_duplicate);
    // Identical content resolves to the existing snapshot and writes nothing.
    let duplicate = fixture
        .workspace
        .inventory_import(edit.clone())
        .await
        .unwrap();
    assert!(duplicate.is_duplicate);
    assert_eq!(duplicate.id, snapshot.id);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inventory_snapshot")
        .fetch_one(&fixture.workspace.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    // Raw files are retained content-addressed.
    let retained: String = sqlx::query_scalar(
        "SELECT relative_path FROM inventory_source_file WHERE snapshot_id = ? AND role = 'stock'",
    )
    .bind(i64::from(snapshot.id))
    .fetch_one(&fixture.workspace.pool)
    .await
    .unwrap();
    assert!(fixture.workspace.path.join(&retained).exists());
    // The tables are append-only.
    let error = sqlx::query("UPDATE inventory_stock SET quantity = 99")
        .execute(&fixture.workspace.pool)
        .await;
    assert!(error.is_err());
    let list = fixture.workspace.inventory_snapshots_list().await.unwrap();
    assert_eq!(list.len(), 1);
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn replenishment_plan_computes_transparent_orders() {
    let fixture = inventory_fixture().await;
    let snapshot = fixture
        .workspace
        .inventory_import(standard_edit(&fixture))
        .await
        .unwrap();
    let plan = fixture
        .workspace
        .replenishment_plan(
            ReplenishmentEdit {
                publication_id: fixture.publication_id,
                snapshot_id: snapshot.id,
                low_multiplier: Some("0.5".into()),
                high_multiplier: Some("2".into()),
            },
            "",
            0,
            50,
        )
        .await
        .unwrap();
    assert_eq!(plan.anchor_week, fixture.horizon_start);
    assert_eq!(plan.horizon_start_week, fixture.horizon_start);
    let stock1 = plan
        .rows
        .iter()
        .find(|row| row.series_key == "STOCK1")
        .unwrap();
    assert!(stock1.blocked.is_empty(), "{:?}", stock1.blocked);
    assert_eq!(stock1.protection_weeks, 3);
    assert_eq!(stock1.opening_stock, 3.0);
    assert_eq!(stock1.inbound_in_window, 5.0);
    // 3 weeks x 10 baseline + 4 safety - 3 stock - 5 inbound = 26 -> moq 10,
    // pack 6: ceil(26/6) = 5 packs = 30.
    assert_eq!(stock1.demand_window, Some(30.0));
    assert_eq!(stock1.raw_need, Some(26.0));
    assert_eq!(stock1.order_base, Some(30.0));
    assert_eq!(stock1.order_low, Some(12.0));
    assert_eq!(stock1.order_high, Some(60.0));
    assert_eq!(stock1.weeks.len(), 12);
    assert_eq!(stock1.weeks[0].closing, Some(-7.0));
    assert!(
        stock1.weeks[2].order_arrival,
        "order arrives after 2 weeks lead"
    );
    let shortage = stock1
        .warnings
        .iter()
        .find(|warning| warning.code == "early_shortage")
        .expect("pre-arrival shortage is disclosed");
    assert!(shortage.detail.contains("cannot repair"));
    // STOCK2 is fully covered: zero need means zero order despite anything.
    let stock2 = plan
        .rows
        .iter()
        .find(|row| row.series_key == "STOCK2")
        .unwrap();
    assert_eq!(stock2.order_base, Some(0.0));
    // SHORT has no published forecast: explicit blocking, never zero-filled.
    let short = plan.rows.iter().find(|row| row.series_key == "SHORT");
    assert!(short.is_none(), "SHORT is not in the inventory batch");
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn replenishment_blocks_overdue_receipts_and_long_protection() {
    let fixture = inventory_fixture().await;
    let as_of_date = NaiveDate::parse_from_str(&fixture.horizon_start, "%Y-%m-%d").unwrap();
    let overdue_week = (as_of_date - Duration::weeks(1))
        .format("%Y-%m-%d")
        .to_string();
    let stock = format!(
        "sku,stock,as_of\nSTOCK1,3,{}\nSTOCK2,100,{}\n",
        fixture.horizon_start, fixture.horizon_start
    );
    let receipts = format!("receipt_id,sku,quantity,available_week\nR2,STOCK2,9,{overdue_week}\n");
    let policy = "sku,unit,lead_weeks,review_weeks,safety_stock,moq,pack_size\n\
                  STOCK1,EA,12,1,0,0,1\n\
                  STOCK2,EA,1,1,0,0,1\n";
    let edit = csv_edit(&fixture, &stock, &receipts, policy);
    let preview = fixture
        .workspace
        .inventory_preview(edit.clone())
        .await
        .unwrap();
    assert!(
        preview.can_import,
        "overdue receipts warn but do not block the import"
    );
    assert_eq!(preview.overdue_receipts, 1);
    let snapshot = fixture.workspace.inventory_import(edit).await.unwrap();
    assert_eq!(snapshot.overdue_receipts, 1);
    let plan = fixture
        .workspace
        .replenishment_plan(
            ReplenishmentEdit {
                publication_id: fixture.publication_id,
                snapshot_id: snapshot.id,
                low_multiplier: None,
                high_multiplier: None,
            },
            "",
            0,
            50,
        )
        .await
        .unwrap();
    let stock1 = plan
        .rows
        .iter()
        .find(|row| row.series_key == "STOCK1")
        .unwrap();
    assert!(stock1
        .blocked
        .iter()
        .any(|finding| finding.code == "protection_too_long"));
    assert_eq!(
        stock1.order_base, None,
        "blocked rows never suggest an order"
    );
    let stock2 = plan
        .rows
        .iter()
        .find(|row| row.series_key == "STOCK2")
        .unwrap();
    assert!(stock2
        .blocked
        .iter()
        .any(|finding| finding.code == "overdue_receipt"));
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn replenishment_blocks_missing_forecast_and_foreign_seriess() {
    let fixture = inventory_fixture().await;
    // A newer revision adds NEWSKU after the publication: its snapshot rows
    // are valid inventory but outside the pinned run.
    let today = Utc::now().date_naive();
    let current_monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let cutoff = current_monday - Duration::weeks(1);
    let weeks = weeks_ending_at(cutoff, 41);
    let mut rows = Vec::new();
    rows.extend(series_rows("STOCK1", &weeks, 10.0));
    rows.extend(series_rows("STOCK2", &weeks, 5.0));
    rows.extend(series_rows("SHORT", &weeks[35..], 4.0));
    rows.extend(series_rows("NEWSKU", &weeks, 7.0));
    let source = fixture.files.join("sales2.csv");
    std::fs::write(&source, canonical(&rows)).unwrap();
    fixture
        .workspace
        .import_csv(&source, &AtomicBool::new(false))
        .await
        .unwrap();
    let stock = format!(
        "sku,stock,as_of\nSTOCK1,3,{}\nNEWSKU,10,{}\n",
        fixture.horizon_start, fixture.horizon_start
    );
    let policy = "sku,unit,lead_weeks,review_weeks,safety_stock,moq,pack_size\n\
                  STOCK1,EA,1,1,0,0,1\n\
                  NEWSKU,EA,1,1,0,0,1\n";
    let snapshot = fixture
        .workspace
        .inventory_import(csv_edit(&fixture, &stock, "", policy))
        .await
        .unwrap();
    let plan = fixture
        .workspace
        .replenishment_plan(
            ReplenishmentEdit {
                publication_id: fixture.publication_id,
                snapshot_id: snapshot.id,
                low_multiplier: None,
                high_multiplier: None,
            },
            "",
            0,
            50,
        )
        .await
        .unwrap();
    let newsku = plan
        .rows
        .iter()
        .find(|row| row.series_key == "NEWSKU")
        .unwrap();
    let mut newsku = newsku.clone();
    assert!(newsku
        .blocked
        .iter()
        .any(|finding| finding.code == "not_in_publication"));
    assert_eq!(newsku.order_base, None);
    // Anchor before the cutoff blocks every row explicitly.
    let earlier = (NaiveDate::parse_from_str(&fixture.horizon_start, "%Y-%m-%d").unwrap()
        - Duration::weeks(1))
    .format("%Y-%m-%d")
    .to_string();
    let stock = format!("sku,stock,as_of\nSTOCK1,3,{earlier}\n");
    let policy =
        "sku,unit,lead_weeks,review_weeks,safety_stock,moq,pack_size\nSTOCK1,EA,1,1,0,0,1\n";
    let snapshot = fixture
        .workspace
        .inventory_import(csv_edit(&fixture, &stock, "", policy))
        .await
        .unwrap();
    let plan = fixture
        .workspace
        .replenishment_plan(
            ReplenishmentEdit {
                publication_id: fixture.publication_id,
                snapshot_id: snapshot.id,
                low_multiplier: None,
                high_multiplier: None,
            },
            "",
            0,
            50,
        )
        .await
        .unwrap();
    let stock1 = plan
        .rows
        .iter()
        .find(|row| row.series_key == "STOCK1")
        .unwrap();
    assert!(stock1
        .blocked
        .iter()
        .any(|finding| finding.code == "anchor_before_horizon"));
    let _ = newsku.blocked.pop();
    fixture.workspace.pool.close().await;
}

#[tokio::test]
async fn replenishment_export_carries_provenance() {
    let fixture = inventory_fixture().await;
    let snapshot = fixture
        .workspace
        .inventory_import(standard_edit(&fixture))
        .await
        .unwrap();
    let edit = ReplenishmentEdit {
        publication_id: fixture.publication_id,
        snapshot_id: snapshot.id,
        low_multiplier: Some("0.5".into()),
        high_multiplier: None,
    };
    let target = fixture.files.join("replenishment.xlsx");
    let result = fixture
        .workspace
        .export_replenishment_xlsx(edit.clone(), &target)
        .await
        .unwrap();
    assert_eq!(result.row_count, 2);
    let mut workbook: calamine::Sheets<BufReader<std::fs::File>> =
        calamine::open_workbook_auto(&target).unwrap();
    use calamine::Reader;
    let summary = workbook.worksheet_range("Summary").unwrap();
    let summary_rows: Vec<Vec<calamine::Data>> = summary.rows().map(|row| row.to_vec()).collect();
    assert_eq!(
        summary_rows[1][1],
        calamine::Data::String("replenishment/1".into())
    );
    assert_eq!(
        summary_rows[6][0],
        calamine::Data::String("publication_id".into())
    );
    match &summary_rows[7][1] {
        calamine::Data::Int(value) => assert_eq!(*value, i64::from(snapshot.id)),
        calamine::Data::Float(value) => assert_eq!(*value, f64::from(snapshot.id)),
        other => panic!("unexpected snapshot id cell: {other:?}"),
    }
    let provenance: (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT publication_id, inventory_snapshot_id FROM export_record ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&fixture.workspace.pool)
    .await
    .unwrap();
    assert_eq!(provenance.0, Some(i64::from(fixture.publication_id)));
    assert_eq!(provenance.1, Some(i64::from(snapshot.id)));
    // Exports never overwrite; multipliers are validated.
    let error = fixture
        .workspace
        .export_replenishment_xlsx(edit.clone(), &target)
        .await
        .unwrap_err();
    assert_eq!(error.code, "export_exists");
    let error = fixture
        .workspace
        .replenishment_plan(
            ReplenishmentEdit {
                low_multiplier: Some("1.5".into()),
                ..edit
            },
            "",
            0,
            50,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_multiplier");
    let error = fixture
        .workspace
        .replenishment_plan(
            ReplenishmentEdit {
                publication_id: 9999,
                snapshot_id: snapshot.id,
                low_multiplier: None,
                high_multiplier: None,
            },
            "",
            0,
            50,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "unknown_publication");
    fixture.workspace.pool.close().await;
}
