<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { getVersion } from '@tauri-apps/api/app';
	import { onMount } from 'svelte';
	import type {
		JobInfo,
		OverrideEdit,
		ParsingDescriptor,
		ReviewActionEdit,
		SeriesNoteEdit,
		SeriesSummary
	} from '#lib/bindings.ts';
	import BrowserFallback from '#lib/components/BrowserFallback.svelte';
	import CsvFormatCard from '#lib/components/CsvFormatCard.svelte';
	import ExceptionInbox from '#lib/components/ExceptionInbox.svelte';
	import HomeScreen from '#lib/components/HomeScreen.svelte';
	import ImportWizard from '#lib/components/ImportWizard.svelte';
	import JobMonitor from '#lib/components/JobMonitor.svelte';
	import RunOverview from '#lib/components/RunOverview.svelte';
	import SeriesDetailPanel from '#lib/components/SeriesDetailPanel.svelte';
	import SeriesList from '#lib/components/SeriesList.svelte';
	import WeeklyComparison from '#lib/components/WeeklyComparison.svelte';
	import WorkspacePanel from '#lib/components/WorkspacePanel.svelte';
	import WorkspacePicker from '#lib/components/WorkspacePicker.svelte';
	import { formatPercent, formatTimestamp, humanizeToken, shortId } from '#lib/frontend/format.ts';
	import { createWorkspaceHint, workspaceActionHint } from '#lib/frontend/home.ts';
	import { resolveLocale, t, type Locale } from '#lib/frontend/i18n.ts';
	import {
		addSeriesNote,
		cancelJob,
		closeWorkspace,
		createWorkspace,
		deleteWorkspace,
		describeError,
		exportPlanCsv,
		exportPlanXlsx,
		exportPublicationCsv,
		exportPublicationXlsx,
		fetchJobStatus,
		fetchJobs,
		fetchPublicationsList,
		fetchRunsList,
		fetchSeriesDetail,
		fetchSeriesList,
		fetchSeriesModels,
		fetchWorkspaceInfo,
		fetchWorkspaces,
		forgetWorkspace,
		isTauriRuntime,
		openWorkspace,
		pickCanonicalCsv,
		pickCsvExport,
		pickWorkspaceDirectory,
		pickXlsxExport,
		publishPlan,
		queryErrorCode,
		renameWorkspace,
		saveOverride,
		setSeriesReview,
		setWorkspaceLocale,
		startForecast,
		startImport
	} from '#lib/frontend/ipc.ts';
	import {
		isActiveJobStatus,
		isTerminalJobStatus,
		jobKindLabel,
		jobStatusLabel,
		selectDisplayRun,
		sortRunsNewestFirst
	} from '#lib/frontend/jobs.ts';
	import { canChangeSelection, selectRunException } from '#lib/frontend/planner.ts';
	import { isNoWorkspaceError, persistLocaleForWorkspace } from '#lib/frontend/workspace.ts';

	const PAGE_SIZE = 50;
	const desktop = isTauriRuntime();
	const queryClient = useQueryClient();
	let appVersion = $state<string | null>(null);

	onMount(() => {
		if (!desktop) return;
		void getVersion()
			.then((version) => (appVersion = version))
			.catch(() => (appVersion = 'Unavailable'));
	});

	let activeJobId = $state<string | null>(null);
	let activeJobStatus = $state<string | null>(null);
	let handledJobId = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let actionError = $state<{ code: string; message: string } | null>(null);
	let selectedSeries = $state<{ productId: string; locationId: string } | null>(null);
	let searchInput = $state('');
	let debouncedSearch = $state('');
	let offset = $state(0);
	let selectedRunId = $state<string | null>(null);
	let plannerDirty = $state(false);
	let plannerSaving = $state(false);
	/** The export format currently in flight; both buttons share one guard. */
	let exportKind = $state<'csv' | 'xlsx' | null>(null);
	const exporting = $derived(exportKind !== null);
	let exportNotice = $state<string | null>(null);
	let publishArmed = $state(false);
	let publishing = $state(false);
	let publicationNotice = $state<string | null>(null);
	let localeSaving = $state(false);
	let seriesScope = '';
	let wizardOpen = $state(false);
	let comparisonOpen = $state(false);
	let inboxOpen = $state(false);
	/** Home flow with no workspace open: module menu, then the workspace list. */
	let homeScreen = $state<'home' | 'picker'>('home');
	const actionErrorHint = $derived(
		actionError
			? createWorkspaceHint(actionError.code) ?? workspaceActionHint(actionError.code)
			: null
	);

	// Debounce the series search; a new search always restarts at page one.
	$effect(() => {
		const timer = setTimeout(() => {
			debouncedSearch = searchInput;
			offset = 0;
		}, 250);
		return () => clearTimeout(timer);
	});

	const workspaceQuery = createQuery(() => ({
		queryKey: ['workspace'],
		queryFn: fetchWorkspaceInfo,
		enabled: desktop,
		retry: 0
	}));
	const workspace = $derived(workspaceQuery.data ?? null);
	const latestRevisionId = $derived(workspace?.latest_revision_id ?? null);
	const locale = $derived(resolveLocale(workspace?.locale));

	const workspacesQuery = createQuery(() => ({
		queryKey: ['workspaces'],
		queryFn: fetchWorkspaces,
		enabled: desktop && workspace === null && homeScreen === 'picker',
		retry: 0
	}));

	const runsQuery = createQuery(() => ({
		queryKey: ['runs', workspace?.path],
		queryFn: () => fetchRunsList(null),
		enabled: desktop && workspace !== null,
		retry: 0
	}));
	const runs = $derived(runsQuery.data ?? []);
	const completedRunCount = $derived(runs.filter((run) => run.status === 'completed').length);
	const newestRun = $derived(runs.length > 0 ? sortRunsNewestFirst(runs)[0] : null);
	const displayRun = $derived(selectDisplayRun(runs, latestRevisionId, selectedRunId));
	const displayRunId = $derived(displayRun?.id ?? null);
	/** The run the overview strip and its exception navigation belong to. */
	const completedRun = $derived(
		displayRun && displayRun.status === 'completed' ? displayRun : null
	);
	const revisionId = $derived(displayRun?.revision_id ?? latestRevisionId);

	const seriesQuery = createQuery(() => ({
		queryKey: ['series', workspace?.path, revisionId, debouncedSearch, offset],
		queryFn: () => fetchSeriesList(revisionId as number, debouncedSearch, offset, PAGE_SIZE),
		enabled: desktop && revisionId !== null,
		retry: 0
	}));

	$effect(() => {
		const scope = `${workspace?.path ?? ''}:${revisionId ?? ''}`;
		if (scope !== seriesScope) {
			seriesScope = scope;
			selectedSeries = null;
			offset = 0;
			plannerDirty = false;
		}
	});

	const detailQuery = createQuery(() => ({
		queryKey: [
			'series-detail',
			workspace?.path,
			revisionId,
			selectedSeries?.productId ?? null,
			selectedSeries?.locationId ?? null,
			displayRunId
		],
		queryFn: () => {
			const selection = selectedSeries;
			if (selection === null) throw new Error('No series selected');
			return fetchSeriesDetail(
				revisionId as number,
				selection.productId,
				selection.locationId,
				displayRunId
			);
		},
		enabled: desktop && revisionId !== null && selectedSeries !== null,
		retry: 0
	}));

	const modelsQuery = createQuery(() => ({
		queryKey: [
			'series-models',
			workspace?.path,
			displayRunId,
			selectedSeries?.productId ?? null,
			selectedSeries?.locationId ?? null
		],
		queryFn: () => {
			const selection = selectedSeries;
			if (selection === null || displayRunId === null) throw new Error('No series selected');
			return fetchSeriesModels(displayRunId, selection.productId, selection.locationId);
		},
		enabled: desktop && displayRunId !== null && selectedSeries !== null,
		retry: 0
	}));

	const publicationsQuery = createQuery(() => ({
		queryKey: ['publications', workspace?.path],
		queryFn: fetchPublicationsList,
		enabled: desktop && workspace !== null,
		retry: 0
	}));
	/** The official publication of the displayed run's cutoff, when one exists. */
	const publishedForRun = $derived(
		(publicationsQuery.data ?? []).find(
			(publication) =>
				completedRun !== null &&
				publication.horizon_start_week === completedRun.horizon_start_week
		) ?? null
	);

	const jobQuery = createQuery(() => ({
		queryKey: ['job', workspace?.path, activeJobId],
		queryFn: () => fetchJobStatus(activeJobId as string),
		enabled: desktop && activeJobId !== null && isActiveJobStatus(activeJobStatus),
		refetchInterval: 900,
		retry: 0
	}));
	const activeJob = $derived(jobQuery.data ?? null);
	const jobBusy = $derived(activeJobId !== null && isActiveJobStatus(activeJobStatus));

	const jobsQuery = createQuery(() => ({
		queryKey: ['jobs', workspace?.path],
		queryFn: fetchJobs,
		enabled: desktop && workspace !== null,
		retry: 0
	}));
	const recentJobs = $derived(
		(jobsQuery.data ?? []).filter((job) => job.id !== activeJobId).slice(0, 5)
	);

	$effect(() => {
		if (activeJobId === null) {
			const pending = (jobsQuery.data ?? []).find((job) => !isTerminalJobStatus(job.status));
			if (pending) {
				activeJobId = pending.id;
				activeJobStatus = pending.status;
			}
		}
	});

	$effect(() => {
		if (selectedSeries === null && !seriesQuery.isFetching) {
			const first = [...(seriesQuery.data ?? [])].sort((a, b) => b.h_length - a.h_length)[0];
			if (first) selectedSeries = { productId: first.product_id, locationId: first.location_id };
		}
	});

	const selectedSummary = $derived.by(() => {
		const selection = selectedSeries;
		if (selection === null) return null;
		return (
			(seriesQuery.data ?? []).find(
				(item) =>
					item.product_id === selection.productId && item.location_id === selection.locationId
			) ?? null
		);
	});

	// Track the polled job; on the terminal transition refresh the authoritative
	// queries (workspace info, runs, series, jobs) and summarize the outcome.
	$effect(() => {
		const job = activeJob;
		if (!job || job.id !== activeJobId) return;
		activeJobStatus = job.status;
		if (isTerminalJobStatus(job.status) && handledJobId !== job.id) {
			handledJobId = job.id;
			void handleJobTerminal(job);
		}
	});

	function summarizeJob(job: JobInfo): string {
		if (job.status === 'failed') {
			return job.error
				? `${jobKindLabel(job.kind)} could not finish. Review the details or try again.`
				: `${jobKindLabel(job.kind)} could not finish. Try again.`;
		}
		if (job.status === 'cancelled') {
			return `${jobKindLabel(job.kind)} was cancelled.`;
		}
		const importResult = job.import_result;
		if (job.kind === 'import' && importResult) {
			if (importResult.is_duplicate) {
				return `This sales history is already in the workspace. Reused import #${importResult.revision_id}.`;
			}
			const issues = importResult.issue_count > 0
				? ` ${importResult.issue_count} items need review.`
				: '';
			return `Imported ${importResult.series_count} series and ${importResult.observation_count} weekly records.${issues}`;
		}
		const forecastResult = job.forecast_result;
		if (job.kind === 'forecast' && forecastResult) {
			return `Forecast complete: ${forecastResult.forecast_count} weekly values for ${forecastResult.series_count} series (${formatPercent(forecastResult.coverage)} series coverage).`;
		}
		return `${jobKindLabel(job.kind)} ${jobStatusLabel(job.status).toLowerCase()}.`;
	}

	async function handleJobTerminal(job: JobInfo) {
		notice = summarizeJob(job);
		const keepSelection = plannerDirty || plannerSaving;
		const importResult = job.kind === 'import' && job.status === 'completed' ? job.import_result : null;
		const forecastResult =
			job.kind === 'forecast' && job.status === 'completed' ? job.forecast_result : null;

		if (importResult && !keepSelection) {
			selectedRunId = null;
			selectedSeries = null;
			offset = 0;
		}
		if (forecastResult && !keepSelection) {
			selectedRunId = forecastResult.run_id;
			plannerDirty = false;
		}

		await Promise.all([
			queryClient.invalidateQueries({ queryKey: ['workspace'] }),
			queryClient.invalidateQueries({ queryKey: ['runs'] }),
			queryClient.invalidateQueries({ queryKey: ['series'] }),
			queryClient.invalidateQueries({ queryKey: ['series-models'] }),
			queryClient.invalidateQueries({ queryKey: ['jobs'] })
		]);
	}

	function beginJob(job: JobInfo) {
		handledJobId = null;
		notice = null;
		actionError = null;
		activeJobStatus = job.status;
		activeJobId = job.id;
	}

	/** Forget every selection and notice that belongs to the open workspace. */
	function resetWorkspaceScopedState() {
		activeJobId = null;
		activeJobStatus = null;
		handledJobId = null;
		notice = null;
		selectedSeries = null;
		selectedRunId = null;
		searchInput = '';
		plannerDirty = false;
		exportNotice = null;
		offset = 0;
		wizardOpen = false;
		comparisonOpen = false;
	}

	async function invalidateWorkspaceQueries() {
		// The workspace query must reset, not just invalidate: after close the
		// refetch fails with no_workspace, and an invalidated query keeps its
		// stale data while erroring, which would leave the workspace view up.
		await Promise.all([
			queryClient.resetQueries({ queryKey: ['workspace'] }),
			queryClient.invalidateQueries({ queryKey: ['workspaces'] })
		]);
	}

	async function handleCreateWorkspace() {
		actionError = null;
		try {
			await createWorkspace(null);
			resetWorkspaceScopedState();
			homeScreen = 'home';
			await invalidateWorkspaceQueries();
		} catch (error) {
			actionError = describeError(error);
		}
	}

	async function handleOpenWorkspace() {
		actionError = null;
		try {
			const path = await pickWorkspaceDirectory();
			if (!path) return;
			await openWorkspace(path);
			resetWorkspaceScopedState();
			homeScreen = 'home';
			await invalidateWorkspaceQueries();
		} catch (error) {
			actionError = describeError(error);
		}
	}

	async function handleOpenRecentWorkspace(path: string) {
		actionError = null;
		try {
			await openWorkspace(path);
			resetWorkspaceScopedState();
			homeScreen = 'home';
			await invalidateWorkspaceQueries();
		} catch (error) {
			actionError = describeError(error);
			await queryClient.invalidateQueries({ queryKey: ['workspaces'] });
		}
	}

	async function handleCloseWorkspace() {
		actionError = null;
		try {
			await closeWorkspace();
			resetWorkspaceScopedState();
			homeScreen = 'home';
			await invalidateWorkspaceQueries();
		} catch (error) {
			actionError = describeError(error);
		}
	}

	async function handleSwitchWorkspace() {
		actionError = null;
		try {
			await closeWorkspace();
			resetWorkspaceScopedState();
			homeScreen = 'picker';
			await invalidateWorkspaceQueries();
		} catch (error) {
			actionError = describeError(error);
		}
	}

	/**
	 * Persist the workspace language. Drafts, the selected run, and parsing
	 * descriptors are untouched: only the workspace query is refreshed, so a
	 * failure keeps the previous choice visible.
	 */
	async function handleLocaleChange(next: Locale) {
		if (localeSaving || next === locale) return;
		actionError = null;
		const workspacePath = workspace?.path;
		if (!workspacePath) return;
		localeSaving = true;
		try {
			await persistLocaleForWorkspace(
				workspacePath,
				next,
				() => workspace?.path,
				setWorkspaceLocale,
				async () => {
					await queryClient.invalidateQueries({ queryKey: ['workspace'] });
				}
			);
		} catch (error) {
			actionError = describeError(error);
		} finally {
			localeSaving = false;
		}
	}

	async function handleForgetWorkspace(path: string) {
		actionError = null;
		try {
			await forgetWorkspace(path);
			await queryClient.invalidateQueries({ queryKey: ['workspaces'] });
		} catch (error) {
			actionError = describeError(error);
		}
	}

	/** Rename a workspace folder; returns false so the picker keeps the form open. */
	async function handleRenameWorkspace(path: string, name: string): Promise<boolean> {
		actionError = null;
		try {
			await renameWorkspace(path, name);
			await queryClient.invalidateQueries({ queryKey: ['workspaces'] });
			return true;
		} catch (error) {
			actionError = describeError(error);
			return false;
		}
	}

	/** Delete a workspace directory after the typed-name confirmation. */
	async function handleDeleteWorkspace(path: string, confirmName: string): Promise<boolean> {
		actionError = null;
		try {
			await deleteWorkspace(path, confirmName);
			await queryClient.invalidateQueries({ queryKey: ['workspaces'] });
			return true;
		} catch (error) {
			actionError = describeError(error);
			return false;
		}
	}

	async function handleImport() {
		actionError = null;
		try {
			const path = await pickCanonicalCsv();
			if (!path) return;
			beginJob(await startImport(path));
		} catch (error) {
			actionError = describeError(error);
		}
	}

	async function handleWizardImport(path: string, descriptor: ParsingDescriptor) {
		actionError = null;
		beginJob(await startImport(path, descriptor));
		wizardOpen = false;
	}

	function handleGuidedImport() {
		if (jobBusy || plannerDirty || plannerSaving || exporting) return;
		actionError = null;
		comparisonOpen = false;
		inboxOpen = false;
		wizardOpen = true;
	}

	function handleWizardClose() {
		wizardOpen = false;
	}

	function handleCloseComparison() {
		comparisonOpen = false;
	}

	async function handleForecast(imputationMethod: string | null) {
		actionError = null;
		if (latestRevisionId === null) return;
		try {
			beginJob(await startForecast(latestRevisionId, imputationMethod));
		} catch (error) {
			actionError = describeError(error);
		}
	}

	async function handleCancel() {
		if (activeJobId === null) return;
		try {
			await cancelJob(activeJobId);
			activeJobStatus = 'cancelling';
		} catch (error) {
			actionError = describeError(error);
		}
	}

	function handleDismissJob() {
		activeJobId = null;
		activeJobStatus = null;
		handledJobId = null;
	}

	function handleSelectSeries(item: SeriesSummary) {
		if (!canChangeSelection({ dirty: plannerDirty, saving: plannerSaving, exporting })) return;
		if (selectedSeries?.productId === item.product_id && selectedSeries.locationId === item.location_id) return;
		plannerDirty = false;
		selectedSeries = { productId: item.product_id, locationId: item.location_id };
	}

	/**
	 * Open one run overview exception under the same run it was read from,
	 * through the same dirty-edit guard as the series list.
	 */
	function handleSelectException(productId: string, locationId: string): boolean {
		if (!completedRun) return false;
		const selection = selectRunException(
			{
				dirty: plannerDirty,
				saving: plannerSaving,
				exporting,
				selectedRunId: selectedRunId ?? displayRunId,
				selectedProductId: selectedSeries?.productId ?? null,
				selectedLocationId: selectedSeries?.locationId ?? null,
				comparisonOpen
			},
			{ runId: completedRun.id, productId, locationId }
		);
		if (!selection) return false;
		// The series detail, the run comparison, and the review inbox share the
		// main area, so the click deliberately leaves those views.
		comparisonOpen = selection.comparisonOpen;
		inboxOpen = false;
		plannerDirty = false;
		selectedRunId = selection.runId;
		selectedSeries = { productId: selection.productId, locationId: selection.locationId };
		return true;
	}

	/** Leave the inbox for the selected series under the inbox's own run. */
	function handleInboxOpenSeries(productId: string, locationId: string) {
		handleSelectException(productId, locationId);
	}

	function handleSelectRun(runId: string) {
		selectedRunId = runId;
		selectedSeries = null;
		plannerDirty = false;
		exportNotice = null;
		actionError = null;
		inboxOpen = false;
		publishArmed = false;
		publicationNotice = null;
		offset = 0;
		searchInput = '';
	}

	async function handleSaveOverride(edit: OverrideEdit) {
		plannerSaving = true;
		exportNotice = null;
		try {
			return await saveOverride(edit);
		} finally {
			plannerSaving = false;
		}
	}

	async function handleAddNote(edit: SeriesNoteEdit) {
		plannerSaving = true;
		try {
			return await addSeriesNote(edit);
		} finally {
			plannerSaving = false;
		}
	}

	async function handleSetReview(edit: ReviewActionEdit) {
		plannerSaving = true;
		try {
			return await setSeriesReview(edit);
		} finally {
			plannerSaving = false;
		}
	}

	async function refreshDetail() {
		const result = await detailQuery.refetch();
		if (result.error) throw describeError(result.error);
		if (!result.data) throw new Error('The selected plan could not be loaded.');
		return result.data;
	}

	async function handleExport() {
		const run = displayRun;
		if (!run || plannerDirty || plannerSaving || exporting) return;
		const workspacePath = workspace?.path;
		exportKind = 'csv';
		actionError = null;
		exportNotice = null;
		try {
			const path = await pickCsvExport(run.id);
			if (!path) return;
			if (workspace?.path !== workspacePath) throw new Error('The workspace changed. Select the run and export again.');
			const result = await exportPlanCsv(run.id, path);
			exportNotice = `Exported ${result.row_count} weekly rows from all series to ${result.path}.`;
		} catch (error) {
			actionError = describeError(error);
		} finally {
			exportKind = null;
			void queryClient.invalidateQueries({ queryKey: ['jobs'] });
		}
	}

	/** The XLSX handoff reuses the same run, dirty guard, and export state. */
	async function handleExportXlsx() {
		const run = displayRun;
		if (!run || plannerDirty || plannerSaving || exporting) return;
		const workspacePath = workspace?.path;
		exportKind = 'xlsx';		actionError = null;
		exportNotice = null;
		try {
			const path = await pickXlsxExport(run.id);
			if (!path) return;
			if (workspace?.path !== workspacePath) throw new Error('The workspace changed. Select the run and export again.');
			const result = await exportPlanXlsx(run.id, path);
			exportNotice = t(locale, 'export.xlsxSaved', { rows: result.row_count, path: result.path });
		} catch (error) {
			actionError = describeError(error);
		} finally {
			exportKind = null;
			void queryClient.invalidateQueries({ queryKey: ['jobs'] });
		}
	}

	/** Cutoff week of a run: one week before its horizon start. */
	function cutoffOf(run: { horizon_start_week: string } | null): string {
		if (!run) return '';
		const date = new Date(`${run.horizon_start_week}T00:00:00Z`);
		date.setUTCDate(date.getUTCDate() - 7);
		return date.toISOString().slice(0, 10);
	}

	function handleArmPublish() {
		if (!completedRun || plannerDirty || plannerSaving || exporting || jobBusy || publishedForRun) {
			return;
		}
		publicationNotice = null;
		publishArmed = true;
	}

	async function handleConfirmPublish() {
		if (!completedRun || publishing) return;
		publishing = true;
		publicationNotice = null;
		try {
			const publication = await publishPlan(completedRun.id);
			publishArmed = false;
			publicationNotice = `Published cutoff ${publication.cutoff_week} as publication #${publication.id} (${publication.covered_series}/${publication.series_count} series covered).`;
			await publicationsQuery.refetch();
		} catch (error) {
			publishArmed = false;
			actionError = describeError(error);
		} finally {
			publishing = false;
		}
	}

	/** Export the pinned published values; the exact save dialog as run exports. */
	async function handleExportPublication(kind: 'csv' | 'xlsx') {
		const publication = publishedForRun;
		if (!publication || exporting) return;
		const workspacePath = workspace?.path;
		exportKind = kind;
		actionError = null;
		exportNotice = null;
		try {
			const path =
				kind === 'csv'
					? await pickCsvExport(`publication-${publication.id}`)
					: await pickXlsxExport(`publication-${publication.id}`);
			if (!path) return;
			if (workspace?.path !== workspacePath) {
				throw new Error('The workspace changed. Select the run and export again.');
			}
			const result =
				kind === 'csv'
					? await exportPublicationCsv(publication.id, path)
					: await exportPublicationXlsx(publication.id, path);
			exportNotice = t(locale, 'publication.exported', {
				rows: result.row_count,
				path: result.path
			});
		} catch (error) {
			actionError = describeError(error);
		} finally {
			exportKind = null;
			void queryClient.invalidateQueries({ queryKey: ['jobs'] });
		}
	}
</script>

{#if !desktop}
	<BrowserFallback />
{:else}
	<a class="skip-link" href="#main">Skip to main content</a>
	<div class="app">
		<header class="topbar">
			<div class="brand">
				<h1 class="brand-name" translate="no">OpenDemand</h1>
				<span class="brand-tag">Explore sales and forecasts</span>
			</div>
			{#if workspace}
				<div class="topbar-meta">
					<button class="btn btn-secondary" onclick={handleCloseWorkspace} disabled={jobBusy || localeSaving}>
						Home
					</button>
				</div>
			{/if}
		</header>

		{#if workspace}
			<div class="workspace-context">
				<WorkspacePanel
					{workspace}
					{appVersion}
					jobBusy={jobBusy || localeSaving || plannerDirty || plannerSaving || exporting}
					{locale}
					localeBusy={localeSaving}
					onSwitch={handleSwitchWorkspace}
					onImport={handleImport}
					onGuidedImport={handleGuidedImport}
					onForecast={handleForecast}
					onLocaleChange={handleLocaleChange}
				/>
			</div>
		{/if}

		<div class="layout" class:single-column={!workspace || revisionId === null}>
			{#if workspace && revisionId !== null}
				<aside class="sidebar" aria-label="Series">
					<SeriesList
						series={seriesQuery.data ?? []}
						loading={seriesQuery.isPending && seriesQuery.isFetching}
						disabled={plannerDirty || plannerSaving || exporting}
						error={seriesQuery.isError
							? { code: queryErrorCode(seriesQuery.error), message: seriesQuery.error.message }
							: null}
						selected={selectedSeries}
						search={searchInput}
						{offset}
						pageSize={PAGE_SIZE}
						onSearch={(value) => (searchInput = value)}
						onSelect={handleSelectSeries}
						onPrev={() => (offset = Math.max(0, offset - PAGE_SIZE))}
						onNext={() => (offset = offset + PAGE_SIZE)}
						onRetry={() => void seriesQuery.refetch()}
					/>
				</aside>
			{/if}

			<main id="main" class="content" tabindex="-1">
				{#if workspace && (activeJob || notice || recentJobs.length > 0)}
					<JobMonitor
						job={activeJob}
						{notice}
						{recentJobs}
						onCancel={handleCancel}
						onDismiss={handleDismissJob}
						onRefresh={() => void jobsQuery.refetch()}
					/>
				{/if}
				{#if workspace && latestRevisionId !== null}
					<section class="panel run-controls" aria-label="Forecast run and export">
						<div class="run-picker">
							<label class="field-label" for="forecast-run">Forecast to explore</label>
							<select id="forecast-run" value={displayRunId ?? ''} disabled={runsQuery.isPending || plannerDirty || plannerSaving || exporting} onchange={(event) => handleSelectRun(event.currentTarget.value)}>
								<option value="">Latest imported history</option>
								{#each sortRunsNewestFirst(runs) as run (run.id)}
									<option value={run.id}>{formatTimestamp(run.created_at)} | Import #{run.revision_id} | {humanizeToken(run.status)}</option>
								{/each}
							</select>
							{#if runsQuery.isPending && runsQuery.isFetching}
								<p class="muted" role="status">Loading forecast runs…</p>
							{:else if runsQuery.isError}
								<div class="banner banner-error" role="alert">
									<p>Could not load runs: {runsQuery.error.message}</p>
									<button class="btn btn-secondary" onclick={() => void runsQuery.refetch()}>Retry</button>
								</div>
							{:else if runs.length === 0}
								<p class="muted">Generate a forecast to explore this sales history.</p>
							{/if}
						</div>
						<div class="export-controls">
							<button class="btn btn-secondary" disabled={!displayRun || displayRun.status !== 'completed' || plannerDirty || plannerSaving || exporting} onclick={handleExport}>{exporting && exportKind === 'csv' ? 'Exporting CSV…' : 'Export CSV'}</button>
							<button class="btn btn-secondary" disabled={!displayRun || displayRun.status !== 'completed' || plannerDirty || plannerSaving || exporting} onclick={handleExportXlsx}>{exporting && exportKind === 'xlsx' ? t(locale, 'export.xlsxBusy') : t(locale, 'export.xlsx')}</button>
							<button class="btn btn-secondary" disabled={jobBusy || plannerDirty || plannerSaving || exporting || completedRunCount < 2} onclick={() => { inboxOpen = false; comparisonOpen = true; }}>Compare runs</button>
							<button class="btn btn-secondary" disabled={jobBusy || plannerDirty || plannerSaving || exporting || !completedRun} onclick={() => { comparisonOpen = false; inboxOpen = true; }}>{t(locale, 'inbox.open')}</button>
							<p class="muted">{plannerDirty ? 'Save or discard edits before switching series, runs, or exporting.' : 'Exports all series in the selected forecast. Compare runs to see what changed between recent imports.'}</p>
						</div>
						<div class="publication-controls">
							{#if publishedForRun}
								<p class="muted" role="status">
									{t(locale, 'publication.published', {
										time: formatTimestamp(publishedForRun.published_at),
										cutoff: publishedForRun.cutoff_week,
										id: publishedForRun.id
									})}
								</p>
								<div class="publication-buttons">
									<button
										class="btn btn-secondary"
										disabled={exporting}
										onclick={() => handleExportPublication('csv')}
									>
										{t(locale, 'publication.exportCsv')}
									</button>
									<button
										class="btn btn-secondary"
										disabled={exporting}
										onclick={() => handleExportPublication('xlsx')}
									>
										{t(locale, 'publication.exportXlsx')}
									</button>
								</div>
							{:else if publishArmed}
								<p class="muted">{t(locale, 'publication.armedHint')}</p>
								<div class="publication-buttons">
									<button class="btn btn-primary" disabled={publishing} onclick={handleConfirmPublish}>
										{publishing
											? 'Publishing…'
											: t(locale, 'publication.arm', { cutoff: cutoffOf(completedRun) })}
									</button>
									<button class="btn btn-secondary" onclick={() => (publishArmed = false)}>
										{t(locale, 'publication.cancel')}
									</button>
								</div>
							{:else}
								<div class="publication-buttons">
									<button
										class="btn btn-secondary"
										disabled={!completedRun || plannerDirty || plannerSaving || exporting || jobBusy}
										onclick={handleArmPublish}
									>
										{t(locale, 'publication.publish')}
									</button>
								</div>
							{/if}
							{#if publicationNotice}
								<p class="banner banner-success" role="status">{publicationNotice}</p>
							{/if}
						</div>
					</section>
				{/if}
				{#if exportNotice}
					<p class="banner banner-success export-notice" role="status">{exportNotice}</p>
				{/if}
				{#if actionError}
					<div class="banner banner-error action-error" role="alert">
						<p>{actionError.message}</p>
						<details class="error-details">
							<summary>Technical details</summary>
							<code>{actionError.code}</code>
						</details>
					</div>
					{#if actionErrorHint}
						<p class="hint muted">{actionErrorHint}</p>
					{/if}
				{/if}

				{#if workspace && completedRun && !wizardOpen}
					<RunOverview
						runId={completedRun.id}
						workspacePath={workspace.path}
						{locale}
						selectionBlocked={plannerDirty || plannerSaving || exporting}
						onSelectSeries={handleSelectException}
					/>
				{/if}

				{#if wizardOpen && workspace}
					<div class="stack">
						<ImportWizard {locale} onImport={handleWizardImport} onClose={handleWizardClose} />
					</div>
				{:else if comparisonOpen && workspace}
					<WeeklyComparison {locale} onClose={handleCloseComparison} />
				{:else if inboxOpen && completedRun}
					<ExceptionInbox
						runId={completedRun.id}
						{locale}
						onOpenSeries={handleInboxOpenSeries}
						onClose={() => (inboxOpen = false)}
					/>
				{:else if workspaceQuery.isPending}
					<p class="muted" role="status">Opening workspace…</p>
				{:else if workspaceQuery.isError && !isNoWorkspaceError(workspaceQuery.error)}
					<div class="banner banner-error" role="alert">
						<p>Could not read the workspace: {workspaceQuery.error.message}</p>
						<button class="btn btn-secondary" onclick={() => void workspaceQuery.refetch()}>
							Retry
						</button>
					</div>
				{:else if !workspace}
					<div class="stack">
						{#if homeScreen === 'home'}
							<HomeScreen
								onPickModule={(id) => {
									if (id === 'forecasting') homeScreen = 'picker';
								}}
							/>
						{:else}
							<WorkspacePicker
								entries={workspacesQuery.data ?? []}
								loading={workspacesQuery.isPending && workspacesQuery.isFetching}
								busy={jobBusy}
								onCreate={handleCreateWorkspace}
								onOpenDirectory={handleOpenWorkspace}
								onOpenPath={handleOpenRecentWorkspace}
								onForget={handleForgetWorkspace}
								onRename={handleRenameWorkspace}
								onDelete={handleDeleteWorkspace}
								onBack={() => (homeScreen = 'home')}
							/>
						{/if}
					</div>
				{:else if revisionId === null}
					<div class="stack">
						<CsvFormatCard />
					</div>
				{:else if selectedSeries === null}
					<section class="panel card">
						<h2>Imported history is ready</h2>
						<p>Select a series on the left to explore its history and forecast.</p>
						{#if runsQuery.isError}
							<div class="banner banner-error" role="alert">
								<p>Could not load runs: {runsQuery.error.message}</p>
								<button class="btn btn-secondary" onclick={() => void runsQuery.refetch()}>
									Retry
								</button>
							</div>
						{:else if newestRun && !displayRun}
							<p class="banner banner-warn">
								The latest forecast attempt did not finish. Choose a completed forecast or generate a new one.
							</p>
						{:else if !newestRun}
							<p class="muted">
								No forecast exists for this imported history yet. Generate one to explore the next 12 weeks.
							</p>
						{/if}
					</section>
				{:else}
					<SeriesDetailPanel
						summary={selectedSummary}
						detail={detailQuery.data ?? null}
						run={displayRun}
						publication={publishedForRun}
						{locale}
						loading={detailQuery.isFetching}
						error={detailQuery.isError
							? { code: queryErrorCode(detailQuery.error), message: detailQuery.error.message }
							: null}
						models={modelsQuery.data ?? null}
						modelsLoading={modelsQuery.isFetching}
						modelsError={modelsQuery.isError
							? { code: queryErrorCode(modelsQuery.error), message: modelsQuery.error.message }
							: null}
						onRetry={() => void detailQuery.refetch()}
						onRetryModels={() => void modelsQuery.refetch()}
						onSave={handleSaveOverride}
						onRefresh={refreshDetail}
						onDirtyChange={(dirty) => {
							plannerDirty = dirty;
							// Pin the run so a background job cannot replace an edited plan.
							if (dirty) selectedRunId = displayRunId;
						}}
						onNote={handleAddNote}
						onReview={handleSetReview}
					/>
				{/if}
			</main>
		</div>
	</div>
{/if}

<style>
	.app {
		min-height: 100vh;
		display: flex;
		flex-direction: column;
	}

	.topbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
		padding: 0.65rem 1.1rem;
		background: var(--navy);
		color: #ffffff;
	}

	.brand {
		display: flex;
		align-items: baseline;
		gap: 0.6rem;
	}

	.brand-name {
		margin: 0;
		font-weight: 700;
		font-size: 1.02rem;
		letter-spacing: -0.01em;
	}

	.brand-tag {
		color: rgba(255, 255, 255, 0.66);
		font-size: 0.82rem;
	}

	.topbar-meta {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-width: 0;
	}

	.workspace-context {
		padding: 1rem 1.5rem 0;
	}

	.layout {
		flex: 1;
		display: grid;
		grid-template-columns: minmax(15rem, 17rem) minmax(0, 1fr);
		gap: 1.1rem;
		padding: 1rem 1.5rem 1.5rem;
		align-items: start;
	}

	.layout.single-column {
		grid-template-columns: minmax(0, 1fr);
	}

	.sidebar {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.content {
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.run-controls {
		display: flex;
		flex-wrap: wrap;
		align-items: start;
		gap: 0.8rem 1.2rem;
		padding: 0.8rem 1rem;
	}

	.run-picker {
		flex: 1;
		min-width: 15rem;
	}

	.run-picker select {
		width: 100%;
		min-height: 36px;
		border: 1px solid var(--border-strong);
		border-radius: var(--radius-sm);
		padding: 0.35rem 0.5rem;
		background: var(--surface);
		color: var(--ink);
		font: inherit;
		font-size: 0.85rem;
	}

	.export-controls {
		align-self: end;
	}

	.run-controls p {
		margin: 0.35rem 0 0;
		font-size: 0.78rem;
	}

	.export-notice {
		overflow-wrap: anywhere;
	}

	.action-error p {
		margin-bottom: 0.35rem;
	}

	.error-details {
		font-size: 0.82rem;
	}

	.error-details code {
		display: block;
		margin-top: 0.25rem;
		font-family: var(--font-mono);
		font-size: 0.8rem;
	}

	.stack {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.card {
		padding: 1.2rem 1.4rem;
	}

	.card p {
		max-width: 72ch;
	}

	@media (max-width: 920px) {
		.layout {
			grid-template-columns: minmax(0, 1fr);
		}

		.workspace-context {
			padding: 0.8rem 1rem 0;
		}

		.layout {
			padding: 0.8rem 1rem 1rem;
		}
	}

	@media (max-width: 560px) {
		.topbar {
			padding: 0.65rem 0.8rem;
		}

		.brand {
			flex-wrap: wrap;
			gap: 0.15rem 0.55rem;
		}
	}
</style>
