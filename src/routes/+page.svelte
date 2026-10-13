<script lang="ts">
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import type { JobInfo, SeriesSummary } from '$lib/bindings';
	import BrowserFallback from '$lib/components/BrowserFallback.svelte';
	import CsvFormatCard from '$lib/components/CsvFormatCard.svelte';
	import JobMonitor from '$lib/components/JobMonitor.svelte';
	import SeriesDetailPanel from '$lib/components/SeriesDetailPanel.svelte';
	import SeriesList from '$lib/components/SeriesList.svelte';
	import StatusChip from '$lib/components/StatusChip.svelte';
	import WorkspacePanel from '$lib/components/WorkspacePanel.svelte';
	import { formatPercent, humanizeToken, shortId } from '$lib/frontend/format';
	import {
		IpcError,
		cancelJob,
		createWorkspace,
		describeError,
		fetchJobStatus,
		fetchJobs,
		fetchRunsList,
		fetchSeriesDetail,
		fetchSeriesList,
		fetchWorkspaceInfo,
		isTauriRuntime,
		openWorkspace,
		pickCanonicalCsv,
		pickWorkspaceDirectory,
		startForecast,
		startImport
	} from '$lib/frontend/ipc';
	import {
		isActiveJobStatus,
		isTerminalJobStatus,
		jobKindLabel,
		jobStatusLabel,
		selectDisplayRun,
		sortRunsNewestFirst
	} from '$lib/frontend/jobs';

	const PAGE_SIZE = 50;
	const desktop = isTauriRuntime();
	const queryClient = useQueryClient();

	let activeJobId = $state<string | null>(null);
	let activeJobStatus = $state<string | null>(null);
	let handledJobId = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let actionError = $state<{ code: string; message: string } | null>(null);
	let selectedSeries = $state<{ productId: string; locationId: string } | null>(null);
	let searchInput = $state('');
	let debouncedSearch = $state('');
	let offset = $state(0);

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
	const revisionId = $derived(workspace?.latest_revision_id ?? null);

	const seriesQuery = createQuery(() => ({
		queryKey: ['series', workspace?.path, revisionId, debouncedSearch, offset],
		queryFn: () => fetchSeriesList(revisionId as number, debouncedSearch, offset, PAGE_SIZE),
		enabled: desktop && revisionId !== null,
		retry: 0
	}));

	const runsQuery = createQuery(() => ({
		queryKey: ['runs', workspace?.path, revisionId],
		queryFn: () => fetchRunsList(revisionId as number),
		enabled: desktop && revisionId !== null,
		retry: 0
	}));
	const runs = $derived(runsQuery.data ?? []);
	const newestRun = $derived(runs.length > 0 ? sortRunsNewestFirst(runs)[0] : null);
	const displayRun = $derived(selectDisplayRun(runs));
	const displayRunId = $derived(displayRun?.id ?? null);

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
				? `${jobKindLabel(job.kind)} failed: ${job.error.message}`
				: `${jobKindLabel(job.kind)} failed.`;
		}
		if (job.status === 'cancelled') {
			return `${jobKindLabel(job.kind)} was cancelled.`;
		}
		const importResult = job.import_result;
		if (job.kind === 'import' && importResult) {
			const duplicate = importResult.is_duplicate
				? ' Identical source content was already imported, so the existing revision was reused.'
				: '';
			return (
				`Imported revision ${importResult.revision_id}: ${importResult.series_count} series, ` +
				`${importResult.observation_count} observations, ${importResult.issue_count} issues; ` +
				`last week ${importResult.last_week}.${duplicate}`
			);
		}
		const forecastResult = job.forecast_result;
		if (job.kind === 'forecast' && forecastResult) {
			return (
				`Forecast run ${shortId(forecastResult.run_id)} persisted: ` +
				`${forecastResult.forecast_count} forecast points across ${forecastResult.series_count} series ` +
				`(${formatPercent(forecastResult.coverage)} coverage).`
			);
		}
		return `${jobKindLabel(job.kind)} ${jobStatusLabel(job.status).toLowerCase()}.`;
	}

	async function handleJobTerminal(job: JobInfo) {
		notice = summarizeJob(job);
		if (job.kind === 'import' && job.status === 'completed' && job.import_result) {
			selectedSeries = null;
			offset = 0;
		}
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: ['workspace'] }),
			queryClient.invalidateQueries({ queryKey: ['runs'] }),
			queryClient.invalidateQueries({ queryKey: ['series'] }),
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

	async function handleCreateWorkspace() {
		actionError = null;
		try {
			await createWorkspace(null);
			activeJobId = null;
			activeJobStatus = null;
			selectedSeries = null;
			offset = 0;
			await queryClient.invalidateQueries({ queryKey: ['workspace'] });
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
			activeJobId = null;
			activeJobStatus = null;
			selectedSeries = null;
			offset = 0;
			searchInput = '';
			await queryClient.invalidateQueries({ queryKey: ['workspace'] });
		} catch (error) {
			actionError = describeError(error);
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

	async function handleForecast() {
		actionError = null;
		if (revisionId === null) return;
		try {
			beginJob(await startForecast(revisionId));
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
		selectedSeries = { productId: item.product_id, locationId: item.location_id };
	}

	function queryErrorCode(error: unknown): string {
		return error instanceof IpcError ? error.code : 'query_failed';
	}
</script>

{#if !desktop}
	<BrowserFallback />
{:else}
	<div class="app">
		<header class="topbar">
			<div class="brand">
				<span class="brand-name">OpenDemand</span>
				<span class="brand-tag">SKU demand workspace</span>
			</div>
			{#if workspace}
				<div class="topbar-meta">
					<span class="path mono" title={workspace.path}>{workspace.path}</span>
					<StatusChip label="schema {workspace.schema_version}" tone="neutral" />
				</div>
			{/if}
		</header>

		<div class="layout">
			<aside class="sidebar" aria-label="Workspace controls">
				<WorkspacePanel
					{workspace}
					{revisionId}
					{jobBusy}
					onCreate={handleCreateWorkspace}
					onOpen={handleOpenWorkspace}
					onImport={handleImport}
					onForecast={handleForecast}
				/>
				<JobMonitor
					job={activeJob}
					{notice}
					{recentJobs}
					onCancel={handleCancel}
					onDismiss={handleDismissJob}
					onRefresh={() => void jobsQuery.refetch()}
				/>
				{#if workspace && revisionId !== null}
					<SeriesList
						series={seriesQuery.data ?? []}
						loading={seriesQuery.isPending && seriesQuery.isFetching}
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
				{/if}
			</aside>

			<main id="main" class="content">
				{#if actionError}
					<p class="banner banner-error" role="alert">
						[{actionError.code}] {actionError.message}
					</p>
				{/if}

				{#if workspaceQuery.isPending}
					<p class="muted" role="status">Opening workspace…</p>
				{:else if workspaceQuery.isError}
					<div class="banner banner-error" role="alert">
						<p>Could not read the workspace: {workspaceQuery.error.message}</p>
						<button class="btn btn-secondary" onclick={() => void workspaceQuery.refetch()}>
							Retry
						</button>
					</div>
				{:else if !workspace}
					<div class="stack">
						<section class="panel card" aria-labelledby="welcome-heading">
							<h2 id="welcome-heading">Open or create a workspace</h2>
							<p>
								A workspace is a folder that holds your SQLite database, retained CSV copies, and
								forecast runs. Create one in the application data directory, or open an existing
								workspace folder.
							</p>
							<div class="row">
								<button class="btn btn-primary" onclick={handleCreateWorkspace}>
									Create workspace
								</button>
								<button class="btn btn-secondary" onclick={handleOpenWorkspace}>
									Open workspace…
								</button>
							</div>
						</section>
						<CsvFormatCard />
					</div>
				{:else if revisionId === null}
					<div class="stack">
						<CsvFormatCard onImport={handleImport} />
					</div>
				{:else if selectedSeries === null}
					<section class="panel card">
						<h2>Revision {revisionId} is ready</h2>
						<p>Select a series on the left to inspect its history and persisted forecast.</p>
						{#if runsQuery.isError}
							<div class="banner banner-error" role="alert">
								<p>Could not load runs: {runsQuery.error.message}</p>
								<button class="btn btn-secondary" onclick={() => void runsQuery.refetch()}>
									Retry
								</button>
							</div>
						{:else if newestRun && !displayRun}
							<p class="banner banner-warn">
								The most recent run ({shortId(newestRun.id)}) has status
								“{humanizeToken(newestRun.status)}”, so no forecast is displayed.
							</p>
						{:else if !newestRun}
							<p class="muted">
								No forecast run exists for this revision yet. Generate a Naive forecast to persist
								the 12-week horizon.
							</p>
						{/if}
					</section>
				{:else}
					<SeriesDetailPanel
						summary={selectedSummary}
						detail={detailQuery.data ?? null}
						run={displayRun}
						loading={detailQuery.isPending && detailQuery.isFetching}
						error={detailQuery.isError
							? { code: queryErrorCode(detailQuery.error), message: detailQuery.error.message }
							: null}
						onRetry={() => void detailQuery.refetch()}
						onClear={() => (selectedSeries = null)}
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

	.path {
		color: rgba(255, 255, 255, 0.82);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 34rem;
	}

	.layout {
		flex: 1;
		display: grid;
		grid-template-columns: minmax(19rem, 21.5rem) 1fr;
		gap: 1rem;
		padding: 1rem;
		align-items: start;
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

	.stack {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 42rem;
	}

	.card {
		padding: 1.2rem 1.4rem;
	}

	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
		margin-top: 0.6rem;
	}

	@media (max-width: 920px) {
		.layout {
			grid-template-columns: 1fr;
		}

		.path {
			max-width: 12rem;
		}
	}
</style>
