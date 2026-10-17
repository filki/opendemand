use crate::{
    error::{AppError, AppResult},
    imputation::ImputationMethod,
    preprocess::ParsingDescriptor,
    recent,
    types::*,
    workspace::{Workspace, DB_FILENAME},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::Row;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
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

fn app_data_directory(app: &AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|_| AppError::new("io_error", "Application data directory is unavailable."))
}

fn default_workspace_path(app: &AppHandle) -> AppResult<PathBuf> {
    app_data_directory(app).map(|directory| directory.join("workspace"))
}

fn registry_file(app: &AppHandle) -> AppResult<PathBuf> {
    app_data_directory(app).map(|directory| recent::file_path(&directory))
}

/// Remember a successfully opened or created workspace in the home screen's
/// list. A failed write never fails the open itself.
fn record_recent(app: &AppHandle, workspace_path: &Path) {
    if let Ok(file) = registry_file(app) {
        let mut entries = recent::load(&file);
        recent::record(&mut entries, workspace_path, Utc::now().to_rfc3339());
        let _ = recent::save(&file, &entries);
    }
}

fn workspace_ref(entry: recent::RecentWorkspace, default_path: Option<&Path>) -> WorkspaceRef {
    let exists = Path::new(&entry.path).join(DB_FILENAME).exists();
    let is_default = default_path
        .map(|default| Path::new(&entry.path) == default)
        .unwrap_or(false);
    WorkspaceRef {
        path: entry.path,
        name: entry.name,
        last_opened_at: entry.last_opened_at,
        exists,
        is_default,
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
        None => default_workspace_path(&app)?,
    };
    let workspace = if default_path && path.exists() {
        Workspace::open(path.clone()).await?
    } else {
        Workspace::create(path.clone()).await?
    };
    let info = workspace.info().await?;
    record_recent(&app, &path);
    state.jobs.lock().unwrap().clear();
    *current = Some(workspace);
    Ok(info)
}

#[tauri::command]
#[specta::specta]
async fn workspace_open(
    path: String,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<WorkspaceInfo> {
    let mut current = state.workspace.lock().await;
    state.ensure_idle()?;
    let workspace = Workspace::open(PathBuf::from(path.clone())).await?;
    let info = workspace.info().await?;
    record_recent(&app, Path::new(&path));
    state.jobs.lock().unwrap().clear();
    *current = Some(workspace);
    Ok(info)
}

#[tauri::command]
#[specta::specta]
async fn workspace_close(state: State<'_, Service>) -> AppResult<()> {
    state.ensure_idle()?;
    *state.workspace.lock().await = None;
    state.jobs.lock().unwrap().clear();
    Ok(())
}

#[tauri::command]
#[specta::specta]
async fn workspace_info(state: State<'_, Service>) -> AppResult<WorkspaceInfo> {
    state.current().await?.info().await
}

#[tauri::command]
#[specta::specta]
async fn workspace_set_locale(
    locale: String,
    state: State<'_, Service>,
) -> AppResult<WorkspaceInfo> {
    let current = state.workspace.lock().await;
    let workspace = current
        .as_ref()
        .ok_or_else(|| AppError::new("no_workspace", "Create or open a workspace first."))?;
    workspace.set_locale(&locale).await
}

/// The home screen's workspace list: recent workspaces plus the default
/// application-data workspace when it exists. The exists flag reflects the
/// workspace database, not just the folder.
#[tauri::command]
#[specta::specta]
async fn workspaces_list(app: AppHandle) -> AppResult<Vec<WorkspaceRef>> {
    let file = registry_file(&app)?;
    let mut entries = recent::load(&file);
    let default = default_workspace_path(&app).ok();
    if let Some(default) = default.as_deref() {
        recent::seed_default(&mut entries, default);
    }
    Ok(entries
        .into_iter()
        .map(|entry| workspace_ref(entry, default.as_deref()))
        .collect())
}

/// Drop one row from the workspace list. The default application-data
/// workspace stays listed while it exists, even after it is forgotten.
#[tauri::command]
#[specta::specta]
async fn workspaces_forget(path: String, app: AppHandle) -> AppResult<()> {
    let file = registry_file(&app)?;
    let mut entries = recent::load(&file);
    recent::forget(&mut entries, &path);
    recent::save(&file, &entries).map_err(|error| {
        AppError::new(
            "io_error",
            format!("Could not update the workspace list: {error}"),
        )
    })
}

/// A workspace name must be a single, usable directory name.
fn workspace_name(value: &str) -> AppResult<String> {
    let name = value.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().count() > 128
        || name.contains(['/', '\\', '\0'])
    {
        return Err(AppError::new(
            "invalid_workspace_name",
            "Use a single folder name without path separators.",
        ));
    }
    Ok(name.to_string())
}

/// The default application-data workspace cannot be renamed or deleted: its
/// path is derived by `default_workspace_path`.
fn refuse_default_workspace(app: &AppHandle, path: &Path) -> AppResult<()> {
    if let Ok(default) = default_workspace_path(app) {
        if path == default {
            return Err(AppError::new(
                "workspace_default",
                "The default workspace cannot be renamed or deleted.",
            ));
        }
    }
    Ok(())
}

/// Rename a workspace by moving its directory and re-pointing the registry
/// entry. Refused for the default workspace, for the currently open workspace,
/// and while a job runs.
#[tauri::command]
#[specta::specta]
async fn workspace_rename(
    path: String,
    name: String,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<WorkspaceRef> {
    let name = workspace_name(&name)?;
    let source = PathBuf::from(&path);
    refuse_default_workspace(&app, &source)?;
    if !source.join(DB_FILENAME).exists() {
        return Err(AppError::new(
            "workspace_not_found",
            "No workspace database was found at this path; create it first.",
        ));
    }
    {
        let current = state.workspace.lock().await;
        if let Some(open) = current.as_ref() {
            if open.path == source {
                return Err(AppError::new(
                    "workspace_busy",
                    "Close the open workspace before renaming it.",
                ));
            }
        }
    }
    let parent = source.parent().ok_or_else(|| {
        AppError::new(
            "invalid_workspace_path",
            "The workspace path has no parent folder.",
        )
    })?;
    let target = parent.join(&name);
    // Share the open/job lock through the filesystem move and registry update.
    let current = state.workspace.lock().await;
    state.ensure_idle()?;
    if let Some(open) = current.as_ref() {
        if open.path == source {
            return Err(AppError::new(
                "workspace_busy",
                "Close the open workspace before renaming it.",
            ));
        }
    }
    if target != source {
        if target.exists() {
            return Err(AppError::new(
                "workspace_name_taken",
                "A folder with that name already exists next to this workspace.",
            ));
        }
        std::fs::rename(&source, &target)?;
    }
    let file = registry_file(&app)?;
    let mut entries = recent::load(&file);
    recent::rename(&mut entries, &source, &target);
    let _ = recent::save(&file, &entries);
    let entry = entries
        .into_iter()
        .find(|entry| Path::new(&entry.path) == target)
        .unwrap_or_else(|| recent::RecentWorkspace::new(&target, String::new()));
    Ok(workspace_ref(entry, None))
}

/// Delete a workspace by removing its entire directory. The caller must pass
/// the current workspace name exactly; the expected name is derived from the
/// path, never taken from the caller. Refused for the default workspace, for
/// the currently open workspace, and while a job runs.
#[tauri::command]
#[specta::specta]
async fn workspace_delete(
    path: String,
    confirm_name: String,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<()> {
    let source = PathBuf::from(&path);
    refuse_default_workspace(&app, &source)?;
    if !source.join(DB_FILENAME).exists() {
        return Err(AppError::new(
            "workspace_not_found",
            "No workspace database was found at this path; nothing to delete.",
        ));
    }
    // Share the open/job lock through the filesystem removal and registry update.
    let current = state.workspace.lock().await;
    state.ensure_idle()?;
    if let Some(open) = current.as_ref() {
        if open.path == source {
            return Err(AppError::new(
                "workspace_busy",
                "Close the open workspace before deleting it.",
            ));
        }
    }
    let expected = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if confirm_name != expected {
        return Err(AppError::new(
            "confirm_name_mismatch",
            "Type the workspace name exactly to confirm deletion.",
        ));
    }
    std::fs::remove_dir_all(&source)?;
    let file = registry_file(&app)?;
    let mut entries = recent::load(&file);
    recent::forget(&mut entries, &path);
    let _ = recent::save(&file, &entries);
    Ok(())
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
async fn series_models(
    run_id: String,
    product_id: String,
    location_id: String,
    state: State<'_, Service>,
) -> AppResult<Vec<SeriesModelComparison>> {
    state
        .current()
        .await?
        .series_models(&run_id, &product_id, &location_id)
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
async fn series_note_add(edit: SeriesNoteEdit, state: State<'_, Service>) -> AppResult<SeriesNote> {
    state.current().await?.series_note_add(edit).await
}

#[tauri::command]
#[specta::specta]
async fn series_review_set(
    edit: ReviewActionEdit,
    state: State<'_, Service>,
) -> AppResult<SeriesReviewInfo> {
    state.current().await?.series_review_set(edit).await
}

#[tauri::command]
#[specta::specta]
async fn review_policy(state: State<'_, Service>) -> AppResult<InboxPolicy> {
    state.current().await?.review_policy().await
}

#[tauri::command]
#[specta::specta]
async fn review_policy_set(
    edit: InboxPolicyEdit,
    state: State<'_, Service>,
) -> AppResult<InboxPolicy> {
    state.current().await?.save_review_policy(edit).await
}

#[tauri::command]
#[specta::specta]
async fn exception_inbox(
    run_id: String,
    search: String,
    filter: String,
    offset: u32,
    limit: u32,
    state: State<'_, Service>,
) -> AppResult<ExceptionInbox> {
    state
        .current()
        .await?
        .exception_inbox(&run_id, &search, &filter, offset, limit)
        .await
}

#[tauri::command]
#[specta::specta]
async fn publish_plan(
    edit: PublicationEdit,
    state: State<'_, Service>,
) -> AppResult<PlanPublication> {
    state.current().await?.publish_plan(edit).await
}

#[tauri::command]
#[specta::specta]
async fn publications_list(state: State<'_, Service>) -> AppResult<Vec<PlanPublication>> {
    state.current().await?.publications_list().await
}

#[tauri::command]
#[specta::specta]
async fn publication_series_detail(
    publication_id: i32,
    product_id: String,
    location_id: String,
    state: State<'_, Service>,
) -> AppResult<PublicationSeriesDetail> {
    state
        .current()
        .await?
        .publication_series_detail(publication_id, &product_id, &location_id)
        .await
}

#[tauri::command]
#[specta::specta]
async fn export_publication_csv(
    publication_id: i32,
    path: String,
    state: State<'_, Service>,
) -> AppResult<ExportResult> {
    state
        .current()
        .await?
        .export_publication_csv(publication_id, std::path::Path::new(&path))
        .await
}

#[tauri::command]
#[specta::specta]
async fn export_publication_xlsx(
    publication_id: i32,
    path: String,
    state: State<'_, Service>,
) -> AppResult<ExportResult> {
    state
        .current()
        .await?
        .export_publication_xlsx(publication_id, std::path::Path::new(&path))
        .await
}

#[tauri::command]
#[specta::specta]
async fn actuals_confirmation_set(
    edit: ActualsConfirmationEdit,
    state: State<'_, Service>,
) -> AppResult<ActualsConfirmation> {
    state.current().await?.actuals_confirmation_set(edit).await
}

#[tauri::command]
#[specta::specta]
async fn actuals_confirmation_latest(
    state: State<'_, Service>,
) -> AppResult<Option<ActualsConfirmation>> {
    state.current().await?.actuals_confirmation_latest().await
}

#[tauri::command]
#[specta::specta]
async fn fva_report(
    horizon: u32,
    search: String,
    offset: u32,
    limit: u32,
    state: State<'_, Service>,
) -> AppResult<FvaReport> {
    state
        .current()
        .await?
        .fva_report(horizon, &search, offset, limit)
        .await
}

#[tauri::command]
#[specta::specta]
async fn fva_publication_series(
    publication_id: i32,
    horizon: u32,
    search: String,
    offset: u32,
    limit: u32,
    state: State<'_, Service>,
) -> AppResult<PublicationFvaPoints> {
    state
        .current()
        .await?
        .fva_publication_series(publication_id, horizon, &search, offset, limit)
        .await
}

#[tauri::command]
#[specta::specta]
async fn inventory_preview(
    edit: InventoryImportEdit,
    state: State<'_, Service>,
) -> AppResult<InventoryPreview> {
    state.current().await?.inventory_preview(edit).await
}

#[tauri::command]
#[specta::specta]
async fn inventory_import(
    edit: InventoryImportEdit,
    state: State<'_, Service>,
) -> AppResult<InventorySnapshot> {
    state.current().await?.inventory_import(edit).await
}

#[tauri::command]
#[specta::specta]
async fn inventory_snapshots_list(state: State<'_, Service>) -> AppResult<Vec<InventorySnapshot>> {
    state.current().await?.inventory_snapshots_list().await
}

#[tauri::command]
#[specta::specta]
async fn replenishment_plan(
    edit: ReplenishmentEdit,
    search: String,
    offset: u32,
    limit: u32,
    state: State<'_, Service>,
) -> AppResult<ReplenishmentPlan> {
    state
        .current()
        .await?
        .replenishment_plan(edit, &search, offset, limit)
        .await
}

#[tauri::command]
#[specta::specta]
async fn export_replenishment_xlsx(
    edit: ReplenishmentEdit,
    path: String,
    state: State<'_, Service>,
) -> AppResult<ExportResult> {
    state
        .current()
        .await?
        .export_replenishment_xlsx(edit, std::path::Path::new(&path))
        .await
}

#[tauri::command]
#[specta::specta]
async fn run_overview(
    run_id: String,
    offset: u32,
    limit: u32,
    state: State<'_, Service>,
) -> AppResult<RunOverview> {
    state
        .current()
        .await?
        .run_overview(&run_id, offset, limit)
        .await
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

#[tauri::command]
#[specta::specta]
async fn export_xlsx(
    run_id: String,
    path: String,
    state: State<'_, Service>,
) -> AppResult<ExportResult> {
    state
        .current()
        .await?
        .export_xlsx(&run_id, &PathBuf::from(path))
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

#[allow(clippy::too_many_arguments)]
async fn queue_job(
    state: &Service,
    app: AppHandle,
    kind: &str,
    source: Option<PathBuf>,
    descriptor: Option<ParsingDescriptor>,
    revision: Option<i32>,
    imputation: Option<ImputationMethod>,
    model_set: Option<String>,
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
                    let path = source.unwrap();
                    let result = match descriptor {
                        Some(descriptor) => {
                            job_workspace.import_raw(&path, &descriptor, &cancel).await
                        }
                        None => job_workspace.import_csv(&path, &cancel).await,
                    };
                    result.map(JobResult::Import)
                } else {
                    let progress_id = job_id.clone();
                    let progress = Arc::new(move |phase: &str, value| {
                        publish(&progress_app, &progress_jobs, &progress_id, phase, value)
                    });
                    crate::forecast::run_forecast_with_model_set(
                        &job_workspace,
                        revision.unwrap(),
                        imputation,
                        model_set.as_deref().unwrap_or("default"),
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
    descriptor: Option<ParsingDescriptor>,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<JobInfo> {
    queue_job(
        &state,
        app,
        "import",
        Some(path.into()),
        descriptor,
        None,
        None,
        None,
    )
    .await
}

/// Detect the shape of a raw export and preview sampled parsed cells. The
/// wizard runs inside an open workspace, but this read-only pass never opens
/// the database.
#[tauri::command]
#[specta::specta]
async fn import_preview(
    path: String,
    descriptor: Option<crate::preprocess::ParsingDescriptor>,
    state: State<'_, Service>,
) -> AppResult<crate::preprocess::ImportPreview> {
    state.current().await?;
    crate::preprocess::preview(Path::new(&path), descriptor.as_ref())
}

#[tauri::command]
#[specta::specta]
async fn import_configs_list(
    state: State<'_, Service>,
) -> AppResult<Vec<crate::preprocess::ImportConfig>> {
    state.current().await?.list_import_configs().await
}

#[tauri::command]
#[specta::specta]
async fn import_config_save(
    name: String,
    descriptor: crate::preprocess::ParsingDescriptor,
    state: State<'_, Service>,
) -> AppResult<crate::preprocess::ImportConfig> {
    state
        .current()
        .await?
        .save_import_config(&name, &descriptor)
        .await
}

#[tauri::command]
#[specta::specta]
async fn import_config_delete(id: i32, state: State<'_, Service>) -> AppResult<()> {
    state.current().await?.delete_import_config(id).await
}

#[tauri::command]
#[specta::specta]
async fn import_config_suggestion(state: State<'_, Service>) -> AppResult<Option<i32>> {
    state.current().await?.import_config_suggestion().await
}

#[tauri::command]
#[specta::specta]
async fn weekly_comparison_context(
    state: State<'_, Service>,
) -> AppResult<WeeklyComparisonContext> {
    state.current().await?.weekly_comparison_context().await
}

#[tauri::command]
#[specta::specta]
async fn weekly_comparison_series_list(
    current_run_id: String,
    previous_run_id: String,
    search: String,
    filter: WeeklyChangeFilter,
    offset: u32,
    limit: u32,
    state: State<'_, Service>,
) -> AppResult<Vec<SeriesChangeSummary>> {
    state
        .current()
        .await?
        .weekly_comparison_series_list(
            &current_run_id,
            &previous_run_id,
            &search,
            filter,
            offset,
            limit,
        )
        .await
}

#[tauri::command]
#[specta::specta]
async fn weekly_comparison_series_detail(
    current_run_id: String,
    previous_run_id: String,
    product_id: String,
    location_id: String,
    state: State<'_, Service>,
) -> AppResult<SeriesComparisonDetail> {
    state
        .current()
        .await?
        .weekly_comparison_series_detail(
            &current_run_id,
            &previous_run_id,
            &product_id,
            &location_id,
        )
        .await
}

#[tauri::command]
#[specta::specta]
async fn forecast_start(
    revision_id: i32,
    imputation: Option<String>,
    model_set: Option<String>,
    app: AppHandle,
    state: State<'_, Service>,
) -> AppResult<JobInfo> {
    if revision_id < 1 {
        return Err(AppError::new(
            "invalid_input",
            "Select a committed import revision.",
        ));
    }
    let imputation = match imputation.as_deref() {
        None => None,
        Some(id) => Some(ImputationMethod::from_id(id).ok_or_else(|| {
            AppError::new(
                "invalid_input",
                format!("Unknown imputation method '{id}'."),
            )
        })?),
    };
    let model_set = model_set.unwrap_or_else(|| "default".to_string());
    crate::forecast::models_for_set(&model_set)?;
    queue_job(
        &state,
        app,
        "forecast",
        None,
        None,
        Some(revision_id),
        imputation,
        Some(model_set),
    )
    .await
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
            workspace_close,
            workspace_info,
            workspace_set_locale,
            workspaces_list,
            workspaces_forget,
            workspace_rename,
            workspace_delete,
            import_start,
            import_preview,
            import_configs_list,
            import_config_save,
            import_config_delete,
            import_config_suggestion,
            weekly_comparison_context,
            weekly_comparison_series_list,
            weekly_comparison_series_detail,
            forecast_start,
            job_status,
            job_cancel,
            jobs_list,
            series_list,
            series_detail,
            series_models,
            runs_list,
            run_overview,
            override_append,
            series_note_add,
            series_review_set,
            review_policy,
            review_policy_set,
            exception_inbox,
            publish_plan,
            publications_list,
            publication_series_detail,
            export_publication_csv,
            export_publication_xlsx,
            actuals_confirmation_set,
            actuals_confirmation_latest,
            fva_report,
            fva_publication_series,
            inventory_preview,
            inventory_import,
            inventory_snapshots_list,
            replenishment_plan,
            export_replenishment_xlsx,
            export_csv,
            export_xlsx
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
            // The desktop shell starts on the home screen; only an explicit
            // environment override opens a workspace automatically (dev and
            // test flows).
            let path = std::env::var_os("OPENDEMAND_WORKSPACE").map(PathBuf::from);
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

    #[test]
    fn workspace_names_are_trimmed_single_folder_names() {
        assert_eq!(
            super::workspace_name("  planner 2026  ").unwrap(),
            "planner 2026"
        );
        assert_eq!(
            super::workspace_name("sklep-kraków").unwrap(),
            "sklep-kraków"
        );
    }

    #[test]
    fn workspace_names_reject_empty_dotted_and_separator_values() {
        for value in ["", "   ", ".", "..", "a/b", "a\\b", "a\0b"] {
            let error = super::workspace_name(value).unwrap_err();
            assert_eq!(error.code, "invalid_workspace_name");
        }
        assert_eq!(
            super::workspace_name(&"x".repeat(129)).unwrap_err().code,
            "invalid_workspace_name"
        );
        assert!(super::workspace_name(&"x".repeat(128)).is_ok());
    }
}
