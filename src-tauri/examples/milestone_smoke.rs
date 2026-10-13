use opendemand_core::{forecast::run_forecast, workspace::Workspace};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 2 {
        return Err("Usage: milestone_smoke NEW_WORKSPACE CANONICAL_CSV".into());
    }
    let workspace = Workspace::create(PathBuf::from(&arguments[0])).await?;
    let imported = workspace
        .import_csv(&PathBuf::from(&arguments[1]), &AtomicBool::new(false))
        .await?;
    let forecast = run_forecast(
        &workspace,
        imported.revision_id,
        "desktop-smoke",
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_, _| {}),
    )
    .await?;
    let series = workspace
        .list_series(imported.revision_id, "", 0, 200)
        .await?;
    let mut details = std::collections::HashMap::new();
    for row in &series {
        let detail = workspace
            .series_detail(
                imported.revision_id,
                &row.product_id,
                &row.location_id,
                Some(&forecast.run_id),
            )
            .await?;
        details.insert(row.product_id.clone(), detail);
    }
    let result = serde_json::json!({
        "workspace": workspace.info().await?,
        "import": imported,
        "forecast": forecast,
        "series": series,
        "runs": workspace.runs_list(imported.revision_id).await?,
        "details": details
    });
    std::fs::write(
        workspace.path.join("smoke-result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!(
        "{}",
        serde_json::json!({"workspace":workspace.path,"series_count":result["forecast"]["series_count"],"forecast_points":result["forecast"]["forecast_count"],"result_file":workspace.path.join("smoke-result.json")})
    );
    workspace.pool.close().await;
    Ok(())
}
