use crate::{
    error::{AppError, AppResult},
    forecast::run_forecast,
    types::*,
    workspace::Workspace,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::Row;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_specta::{collect_commands, collect_events, Builder, ErrorHandlingMode};

#[derive(Clone)]
struct JobEntry {
    info: JobInfo,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Service {
    workspace: tokio::sync::Mutex<Option<Workspace>>,
    jobs: Arc<Mutex<HashMap<String, JobEntry>>>,
    startup_error: Mutex<Option<AppError>>,
    import_active: Arc<AtomicBool>,
    forecast_active: Arc<AtomicBool>,
}

#[derive(Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
struct JobProgress(JobInfo);

impl Service {
    async fn current(&self) -> AppResult<Workspace> {
        self.workspace.lock().await.clone().ok_or_else(|| {
            self.startup_error
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| {
                    AppError::new("no_workspace", "Create or open a workspace first.")
                })
        })
    }

    fn ensure_idle(&self) -> AppResult<()> {
        if self.import_active.load(Ordering::SeqCst) || self.forecast_active.load(Ordering::SeqCst)
        {
            Err(AppError::new(
                "workspace_busy",
                "Finish or cancel the active job before opening another workspace.",
            ))
        } else {
            Ok(())
        }
    }
}

#[tauri::command]
#[specta::specta]
async fn workspace_create(
    path: Option<String>,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<WorkspaceInfo> {
    let mut current = state.workspace.lock().await;
    state.ensure_idle()?;
    let default_path = path.is_none();
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => app
            .path()
            .app_data_dir()
            .map_err(|_| AppError::new("io_error", "Application data directory is unavailable."))?
            .join("workspace"),
    };
    let workspace = if default_path && path.exists() {
        Workspace::open(path).await?
    } else {
        Workspace::create(path).await?
    };
    let info = workspace.info().await?;
    state.jobs.lock().unwrap().clear();
    *current = Some(workspace);
    Ok(info)
}

#[tauri::command]
#[specta::specta]
async fn workspace_open(path: String, state: State<'_, Service>) -> AppResult<WorkspaceInfo> {
    let mut current = state.workspace.lock().await;
    state.ensure_idle()?;
    let workspace = Workspace::open(PathBuf::from(path)).await?;
    let info = workspace.info().await?;
    state.jobs.lock().unwrap().clear();
    *current = Some(workspace);
    Ok(info)
}

#[tauri::command]
#[specta::specta]
async fn workspace_info(state: State<'_, Service>) -> AppResult<WorkspaceInfo> {
    state.current().await?.info().await
}

#[tauri::command]
#[specta::specta]
async fn series_list(
    revision_id: i32,
    search: String,
    offset: u32,
    limit: u32,
    state: State<'_, Service>,
) -> AppResult<Vec<SeriesSummary>> {
    state
        .current()
        .await?
        .list_series(revision_id, &search, offset, limit)
        .await
}

#[tauri::command]
#[specta::specta]
async fn series_detail(
    revision_id: i32,
    product_id: String,
    location_id: String,
    run_id: Option<String>,
    state: State<'_, Service>,
) -> AppResult<SeriesDetail> {
    state
        .current()
        .await?
        .series_detail(revision_id, &product_id, &location_id, run_id.as_deref())
        .await
}

#[tauri::command]
#[specta::specta]
async fn runs_list(
    revision_id: Option<i32>,
    state: State<'_, Service>,
) -> AppResult<Vec<RunSummary>> {
    let workspace = state.current().await?;
    match revision_id {
        Some(revision) => workspace.runs_list(revision).await,
        None => workspace.runs_list_all().await,
    }
}

#[tauri::command]
#[specta::specta]
async fn override_append(
    edit: OverrideEdit,
    state: State<'_, Service>,
) -> AppResult<OverrideRevision> {
    state.current().await?.append_override(edit).await
}

#[tauri::command]
#[specta::specta]
async fn export_csv(
    run_id: String,
    path: String,
    state: State<'_, Service>,
) -> AppResult<ExportResult> {
    state
        .current()
        .await?
        .export_csv(&run_id, &PathBuf::from(path))
        .await
}

fn publish(
    app: &AppHandle,
    jobs: &Mutex<HashMap<String, JobEntry>>,
    id: &str,
    phase: &str,
    progress: f64,
) {
    let info = {
        let mut jobs = jobs.lock().unwrap();
        jobs.get_mut(id).map(|entry| {
            entry.info.phase = phase.into();
            entry.info.progress = progress.clamp(0.0, 1.0);
            entry.info.clone()
        })
    };
    if let Some(info) = info {
        let _ = app.emit("job-progress", info);
    }
}

enum JobResult {
    Import(ImportResult),
    Forecast(ForecastResult),
}

/// Resolve the sidecar executable: explicit override, then the bundled
/// resource shipped inside the desktop application, then the dev fallback.
fn find_sidecar(
    resource_dir: Option<&std::path::Path>,
    env_override: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = env_override {
        return Some(path);
    }
    let name = if cfg!(windows) {
        "opendemand-engine.exe"
    } else {
        "opendemand-engine"
    };
    let resource = resource_dir?;
    [
        resource.join("engine").join(name),
        resource.join("engine").join("opendemand-engine").join(name),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn sidecar_binary(app: &AppHandle) -> Option<PathBuf> {
    find_sidecar(
        app.path().resource_dir().ok().as_deref(),
        std::env::var_os("OPENDEMAND_ENGINE").map(PathBuf::from),
    )
}

async fn queue_job(
    state: &Service,
    app: AppHandle,
    kind: &str,
    source: Option<PathBuf>,
    revision: Option<i32>,
) -> AppResult<JobInfo> {
    let workspace_guard = state.workspace.lock().await;
    let workspace = workspace_guard
        .clone()
        .ok_or_else(|| AppError::new("no_workspace", "Create or open a workspace first."))?;
    let active = if kind == "import" {
        state.import_active.clone()
    } else {
        state.forecast_active.clone()
    };
    active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| AppError::new("job_busy", format!("A {kind} job is already active.")))?;
    let created = sqlx::query("INSERT INTO job (kind, status, created_at) VALUES (?, 'queued', ?)")
        .bind(kind)
        .bind(Utc::now().to_rfc3339())
        .execute(&workspace.pool)
        .await;
    let id = match created {
        Ok(row) => row.last_insert_rowid().to_string(),
        Err(error) => {
            active.store(false, Ordering::SeqCst);
            return Err(error.into());
        }
    };
    let info = JobInfo {
        id: id.clone(),
        kind: kind.into(),
        status: "queued".into(),
        phase: "queued".into(),
        progress: 0.0,
        error: None,
        import_result: None,
        forecast_result: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    state.jobs.lock().unwrap().insert(
        id.clone(),
        JobEntry {
            info: info.clone(),
            cancel: cancel.clone(),
        },
    );
    drop(workspace_guard);
    let jobs = state.jobs.clone();
    let kind = kind.to_owned();
    let engine = sidecar_binary(&app);
    tauri::async_runtime::spawn(async move {
        let started = sqlx::query("UPDATE job SET status='running', started_at=? WHERE id=?")
            .bind(Utc::now().to_rfc3339())
            .bind(id.parse::<i64>().unwrap())
            .execute(&workspace.pool)
            .await;
        if let Some(entry) = jobs.lock().unwrap().get_mut(&id) {
            entry.info.status = "running".into();
        }
        publish(&app, &jobs, &id, "preparing", 0.0);
        let job_workspace = workspace.clone();
        let job_id = id.clone();
        let progress_jobs = jobs.clone();
        let progress_app = app.clone();
        let result = if let Err(error) = started {
            Err(AppError::from(error))
        } else {
            tokio::spawn(async move {
                if kind == "import" {
                    publish(&progress_app, &progress_jobs, &job_id, "importing", 0.1);
                    job_workspace
                        .import_csv(&source.unwrap(), &cancel)
                        .await
                        .map(JobResult::Import)
                } else {
                    let progress_id = job_id.clone();
                    let progress = Arc::new(move |phase: &str, value| {
                        publish(&progress_app, &progress_jobs, &progress_id, phase, value)
                    });
                    run_forecast(
                        &job_workspace,
                        revision.unwrap(),
                        &job_id,
                        cancel,
                        progress,
                        engine,
                    )
                    .await
                    .map(JobResult::Forecast)
                }
            })
            .await
            .unwrap_or_else(|_| {
                Err(AppError::new(
                    "internal_error",
                    "Background job stopped unexpectedly. No partial results were accepted.",
                ))
            })
        };
        let status = match &result {
            Ok(_) => "completed",
            Err(error) if error.code == "cancelled" => "cancelled",
            Err(_) => "failed",
        };
        let error = result.as_ref().err().cloned();
        let _ = sqlx::query(
            "UPDATE job SET status=?, finished_at=?, error_code=?, error_detail=? WHERE id=?",
        )
        .bind(status)
        .bind(Utc::now().to_rfc3339())
        .bind(error.as_ref().map(|error| &error.code))
        .bind(
            error
                .as_ref()
                .map(|error| serde_json::to_string(error).unwrap()),
        )
        .bind(id.parse::<i64>().unwrap())
        .execute(&workspace.pool)
        .await;
        let terminal = {
            let mut jobs = jobs.lock().unwrap();
            jobs.get_mut(&id).map(|entry| {
                entry.info.status = status.into();
                entry.info.phase = status.into();
                entry.info.error = error;
                if let Ok(result) = result {
                    entry.info.progress = 1.0;
                    match result {
                        JobResult::Import(result) => entry.info.import_result = Some(result),
                        JobResult::Forecast(result) => entry.info.forecast_result = Some(result),
                    }
                }
                entry.info.clone()
            })
        };
        active.store(false, Ordering::SeqCst);
        if let Some(info) = terminal {
            let _ = app.emit("job-progress", info);
        }
    });
    Ok(info)
}

#[tauri::command]
#[specta::specta]
async fn import_start(
    path: String,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<JobInfo> {
    queue_job(&state, app, "import", Some(path.into()), None).await
}

#[tauri::command]
#[specta::specta]
async fn forecast_start(
    revision_id: i32,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<JobInfo> {
    if revision_id < 1 {
        return Err(AppError::new(
            "invalid_input",
            "Select a committed import revision.",
        ));
    }
    queue_job(&state, app, "forecast", None, Some(revision_id)).await
}

fn saved_job(row: sqlx::sqlite::SqliteRow) -> JobInfo {
    let code: Option<String> = row.get("error_code");
    let detail: Option<String> = row.get("error_detail");
    let status: String = row.get("status");
    JobInfo {
        id: row.get::<i64, _>("id").to_string(),
        kind: row.get("kind"),
        phase: status.clone(),
        progress: if status == "completed" { 1.0 } else { 0.0 },
        status,
        error: code.map(|code| {
            detail
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok())
                .unwrap_or_else(|| {
                    AppError::new(
                        code,
                        detail.unwrap_or_else(|| "Job did not complete.".into()),
                    )
                })
        }),
        import_result: None,
        forecast_result: None,
    }
}

#[tauri::command]
#[specta::specta]
async fn job_status(job_id: String, state: State<'_, Service>) -> AppResult<JobInfo> {
    if let Some(entry) = state.jobs.lock().unwrap().get(&job_id).cloned() {
        return Ok(entry.info);
    }
    let id = job_id
        .parse::<i64>()
        .map_err(|_| AppError::new("invalid_input", "Invalid job id."))?;
    let workspace = state.current().await?;
    sqlx::query("SELECT * FROM job WHERE id=?")
        .bind(id)
        .fetch_optional(&workspace.pool)
        .await?
        .map(saved_job)
        .ok_or_else(|| AppError::new("not_found", "Job does not exist in this workspace."))
}

#[tauri::command]
#[specta::specta]
async fn jobs_list(state: State<'_, Service>) -> AppResult<Vec<JobInfo>> {
    let workspace = state.current().await?;
    let rows = sqlx::query("SELECT * FROM job ORDER BY id DESC LIMIT 50")
        .fetch_all(&workspace.pool)
        .await?;
    let jobs = state.jobs.lock().unwrap();
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = row.get::<i64, _>("id").to_string();
            jobs.get(&id)
                .map(|entry| entry.info.clone())
                .unwrap_or_else(|| saved_job(row))
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
async fn job_cancel(job_id: String, state: State<'_, Service>) -> AppResult<JobInfo> {
    {
        let mut jobs = state.jobs.lock().unwrap();
        if let Some(entry) = jobs.get_mut(&job_id) {
            if ["queued", "running"].contains(&entry.info.status.as_str()) {
                entry.cancel.store(true, Ordering::SeqCst);
                entry.info.phase = "cancelling".into();
            }
            return Ok(entry.info.clone());
        }
    }
    job_status(job_id, state).await
}

fn command_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .error_handling(ErrorHandlingMode::Result)
        .commands(collect_commands![
            workspace_create,
            workspace_open,
            workspace_info,
            import_start,
            forecast_start,
            job_status,
            job_cancel,
            jobs_list,
            series_list,
            series_detail,
            runs_list,
            override_append,
            export_csv
        ])
        .events(collect_events![JobProgress])
}

pub fn run() {
    let builder = command_builder();
    #[cfg(debug_assertions)]
    {
        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/lib/bindings.ts");
        std::fs::create_dir_all(output.parent().unwrap()).expect("create bindings directory");
        builder
            .export(specta_typescript::Typescript::default(), output)
            .expect("export typed IPC");
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Service::default())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let path = std::env::var_os("OPENDEMAND_WORKSPACE")
                .map(PathBuf::from)
                .or_else(|| {
                    app.path()
                        .app_data_dir()
                        .ok()
                        .map(|root| root.join("workspace"))
                        .filter(|path| path.exists())
                });
            if let Some(path) = path {
                match tauri::async_runtime::block_on(Workspace::open(path)) {
                    Ok(workspace) => tauri::async_runtime::block_on(async {
                        *app.state::<Service>().workspace.lock().await = Some(workspace);
                    }),
                    Err(error) => {
                        *app.state::<Service>().startup_error.lock().unwrap() = Some(error)
                    }
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("OpenDemand desktop runtime failed");
}

#[cfg(test)]
mod tests {
    #[test]
    fn export_generated_frontend_bindings() {
        let path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/lib/bindings.ts");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        super::command_builder()
            .export(specta_typescript::Typescript::default(), path)
            .unwrap();
    }

    #[test]
    fn sidecar_resolution_prefers_override_then_bundled_resource() {
        let name = if cfg!(windows) {
            "opendemand-engine.exe"
        } else {
            "opendemand-engine"
        };
        let directory = tempfile::tempdir().unwrap();

        let flat_root = directory.path().join("flat");
        let flat = flat_root.join("engine");
        std::fs::create_dir_all(&flat).unwrap();
        std::fs::write(flat.join(name), b"engine").unwrap();
        let override_path = directory.path().join("custom-engine");
        assert_eq!(
            super::find_sidecar(Some(flat_root.as_path()), Some(override_path.clone())),
            Some(override_path)
        );
        assert_eq!(
            super::find_sidecar(Some(flat_root.as_path()), None),
            Some(flat.join(name))
        );

        let nested_root = directory.path().join("nested");
        let nested = nested_root.join("engine").join("opendemand-engine");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(name), b"engine").unwrap();
        assert_eq!(
            super::find_sidecar(Some(nested_root.as_path()), None),
            Some(nested.join(name))
        );

        assert_eq!(super::find_sidecar(Some(directory.path()), None), None);
        assert_eq!(super::find_sidecar(None, None), None);
    }
}
