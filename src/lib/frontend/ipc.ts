/**
 * Typed wrapper over the generated tauri-specta bindings. `src/lib/bindings.ts`
 * is generated from the Rust command collection and is the source of truth;
 * this module only unwraps the Result envelope, exposes the dialog pickers,
 * and detects whether the page runs inside the Tauri desktop shell.
 */
import { open, save } from '@tauri-apps/plugin-dialog';
import { commands } from '#lib/bindings.ts';
import type {
	AppError,
	ExportResult,
	ImportConfig,
	ImportPreview,
	JobInfo,
	OverrideEdit,
	OverrideRevision,
	ParsingDescriptor,
	RunSummary,
	SeriesChangeSummary,
	SeriesComparisonDetail,
	SeriesDetail,
	SeriesModelComparison,
	SeriesSummary,
	WeeklyComparisonContext,
	WorkspaceInfo
} from '#lib/bindings.ts';

export type IpcOutcome<T> = { status: 'ok'; data: T } | { status: 'error'; error: AppError };

export class IpcError extends Error {
	readonly code: string;

	constructor(error: AppError) {
		super(error.message);
		this.name = 'IpcError';
		this.code = error.code;
	}
}

export function unwrap<T>(outcome: IpcOutcome<T>): T {
	if (outcome.status === 'ok') return outcome.data;
	throw new IpcError(outcome.error);
}

export function describeError(error: unknown): { code: string; message: string } {
	if (error instanceof IpcError) return { code: error.code, message: error.message };
	if (error instanceof Error) return { code: 'unexpected_error', message: error.message };
	return { code: 'unexpected_error', message: 'Unexpected error.' };
}

/** Query-error code for banners: the IPC code when the failure crossed IPC. */
export function queryErrorCode(error: unknown): string {
	return error instanceof IpcError ? error.code : 'query_failed';
}

interface TauriScope {
	__TAURI_INTERNALS__?: unknown;
	__TAURI__?: unknown;
}

/**
 * True only when a Tauri runtime is present (desktop shell or a test that
 * injects the runtime). The parameter keeps the check unit-testable; the app
 * always calls it without arguments.
 */
export function isTauriRuntime(scope: unknown = globalThis): boolean {
	if (typeof scope !== 'object' || scope === null) return false;
	const candidate = scope as TauriScope;
	if (typeof candidate.__TAURI_INTERNALS__ === 'object' && candidate.__TAURI_INTERNALS__ !== null) {
		return true;
	}
	return typeof candidate.__TAURI__ === 'object' && candidate.__TAURI__ !== null;
}

export async function fetchWorkspaceInfo(): Promise<WorkspaceInfo> {
	return unwrap(await commands.workspaceInfo());
}

export async function createWorkspace(path: string | null): Promise<WorkspaceInfo> {
	return unwrap(await commands.workspaceCreate(path));
}

export async function openWorkspace(path: string): Promise<WorkspaceInfo> {
	return unwrap(await commands.workspaceOpen(path));
}

export async function startImport(
	path: string,
	descriptor: ParsingDescriptor | null = null
): Promise<JobInfo> {
	return unwrap(await commands.importStart(path, descriptor));
}

export async function previewImport(
	path: string,
	descriptor: ParsingDescriptor | null
): Promise<ImportPreview> {
	return unwrap(await commands.importPreview(path, descriptor));
}

export async function fetchImportConfigs(): Promise<ImportConfig[]> {
	return unwrap(await commands.importConfigsList());
}

export async function saveImportConfig(
	name: string,
	descriptor: ParsingDescriptor
): Promise<ImportConfig> {
	return unwrap(await commands.importConfigSave(name, descriptor));
}

export async function deleteImportConfig(id: number): Promise<void> {
	await unwrap(await commands.importConfigDelete(id));
}

export async function fetchImportConfigSuggestion(): Promise<number | null> {
	return unwrap(await commands.importConfigSuggestion());
}

export async function fetchWeeklyComparisonContext(): Promise<WeeklyComparisonContext> {
	return unwrap(await commands.weeklyComparisonContext());
}

export async function fetchWeeklyComparisonList(
	currentRunId: string,
	previousRunId: string,
	search: string,
	offset: number,
	limit: number
): Promise<SeriesChangeSummary[]> {
	return unwrap(
		await commands.weeklyComparisonSeriesList(currentRunId, previousRunId, search, offset, limit)
	);
}

export async function fetchWeeklyComparisonDetail(
	currentRunId: string,
	previousRunId: string,
	productId: string,
	locationId: string
): Promise<SeriesComparisonDetail> {
	return unwrap(
		await commands.weeklyComparisonSeriesDetail(currentRunId, previousRunId, productId, locationId)
	);
}

export async function startForecast(revisionId: number): Promise<JobInfo> {
	return unwrap(await commands.forecastStart(revisionId));
}

export async function fetchJobStatus(jobId: string): Promise<JobInfo> {
	return unwrap(await commands.jobStatus(jobId));
}

export async function cancelJob(jobId: string): Promise<void> {
	await unwrap(await commands.jobCancel(jobId));
}

export async function fetchJobs(): Promise<JobInfo[]> {
	return unwrap(await commands.jobsList());
}

export async function fetchSeriesList(
	revisionId: number,
	search: string,
	offset: number,
	limit: number
): Promise<SeriesSummary[]> {
	return unwrap(await commands.seriesList(revisionId, search, offset, limit));
}

export async function fetchSeriesDetail(
	revisionId: number,
	productId: string,
	locationId: string,
	runId: string | null
): Promise<SeriesDetail> {
	return unwrap(await commands.seriesDetail(revisionId, productId, locationId, runId));
}

export async function fetchSeriesModels(
	runId: string,
	productId: string,
	locationId: string
): Promise<SeriesModelComparison[]> {
	return unwrap(await commands.seriesModels(runId, productId, locationId));
}

export async function fetchRunsList(revisionId: number | null): Promise<RunSummary[]> {
	return unwrap(await commands.runsList(revisionId));
}

export async function saveOverride(edit: OverrideEdit): Promise<OverrideRevision> {
	return unwrap(await commands.overrideAppend(edit));
}

export async function exportPlanCsv(runId: string, path: string): Promise<ExportResult> {
	return unwrap(await commands.exportCsv(runId, path));
}

export async function pickCsvExport(runId: string): Promise<string | null> {
	const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
	return save({
		title: 'Export saved plan to CSV',
		defaultPath: `opendemand-${runId}-${timestamp}.csv`,
		filters: [{ name: 'Planning CSV', extensions: ['csv'] }]
	});
}

function firstSelectedPath(selection: unknown): string | null {
	const first = Array.isArray(selection) ? selection[0] : selection;
	if (typeof first === 'string') return first;
	if (first !== null && typeof first === 'object' && 'path' in first) {
		const path = (first as { path: unknown }).path;
		if (typeof path === 'string') return path;
	}
	return null;
}

export async function pickWorkspaceDirectory(): Promise<string | null> {
	const selection = await open({ directory: true, multiple: false, title: 'Choose a workspace folder' });
	return firstSelectedPath(selection);
}

export async function pickCanonicalCsv(): Promise<string | null> {
	const selection = await open({
		multiple: false,
		title: 'Import canonical CSV',
		filters: [{ name: 'Canonical CSV', extensions: ['csv'] }]
	});
	return firstSelectedPath(selection);
}

export async function pickImportSource(): Promise<string | null> {
	const selection = await open({
		multiple: false,
		title: 'Choose a planner export',
		filters: [{ name: 'Planner exports', extensions: ['csv', 'txt', 'xlsx'] }]
	});
	return firstSelectedPath(selection);
}
