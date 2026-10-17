<script lang="ts">
	import type {
		OverrideEdit,
		OverrideRevision,
		ReviewActionEdit,
		RunSummary,
		SeriesDetail,
		SeriesModelComparison,
		SeriesNote,
		SeriesNoteEdit,
		SeriesSummary
	} from '#lib/bindings.ts';
	import ForecastEditor from '#lib/components/ForecastEditor.svelte';
	import ModelComparison from '#lib/components/ModelComparison.svelte';
	import SeriesChart from '#lib/components/SeriesChart.svelte';
	import SeriesReviewPanel from '#lib/components/SeriesReviewPanel.svelte';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import {
		formatScore,
		formatTimestamp,
		humanizeToken,
		imputationLabel,
		outcomeLabel,
		qualificationLabel,
		shortId
	} from '#lib/frontend/format.ts';

	let {
		summary,
		detail,
		run,
		loading,
		error,
		models,
		modelsLoading,
		modelsError,
		onRetry,
		onRetryModels,
		onSave,
		onRefresh,
		onDirtyChange,
		onNote,
		onReview
	}: {
		summary: SeriesSummary | null;
		detail: SeriesDetail | null;
		run: RunSummary | null;
		loading: boolean;
		error: { code: string; message: string } | null;
		models: SeriesModelComparison[] | null;
		modelsLoading: boolean;
		modelsError: { code: string; message: string } | null;
		onRetry: () => void;
		onRetryModels: () => void;
		onSave: (edit: OverrideEdit) => Promise<OverrideRevision>;
		onRefresh: () => Promise<SeriesDetail>;
		onDirtyChange: (dirty: boolean) => void;
		onNote: (edit: SeriesNoteEdit) => Promise<SeriesNote>;
		onReview: (edit: ReviewActionEdit) => Promise<unknown>;
	} = $props();

	const seriesKey = $derived(summary?.series_key ?? detail?.series_key ?? 'Series');
	const productId = $derived(summary?.product_id ?? detail?.product_id ?? '');
	const locationId = $derived(summary?.location_id ?? detail?.location_id ?? '');

	function outcomeTone(outcome: string | null): 'ok' | 'info' | 'warn' | 'error' | 'neutral' {
		switch (outcome) {
			case 'ok':
				return 'ok';
			case 'fallback_naive':
				return 'info';
			case 'insufficient_history':
				return 'warn';
			case 'model_failure':
			case 'invalid_data':
				return 'error';
			default:
				return 'neutral';
		}
	}

	function qualificationTone(status: string): 'ok' | 'warn' | 'error' | 'neutral' {
		switch (status) {
			case 'eligible':
				return 'ok';
			case 'insufficient_history':
				return 'warn';
			case 'invalid_data':
				return 'error';
			default:
				return 'neutral';
		}
	}

	function runTone(status: string): 'ok' | 'error' | 'neutral' {
		if (status === 'completed') return 'ok';
		if (status === 'failed') return 'error';
		return 'neutral';
	}
</script>

<section class="detail" aria-labelledby="series-detail-heading">
	<header class="head">
		<div class="head-main">
			<h2 id="series-detail-heading" class="key" title={seriesKey}>{seriesKey}</h2>
			<p class="ids muted mono">
				<span title={productId}>product {shortId(productId, 13)}</span>
				<span title={locationId}>location {summary?.location_code ?? detail?.location_code ?? shortId(locationId, 13)}</span>
			</p>
		</div>
	</header>

	{#if error}
		<div class="banner banner-error" role="alert">
			<p>[{error.code}] {error.message}</p>
			<button class="btn btn-secondary btn-small" onclick={onRetry}>Retry</button>
		</div>
	{/if}
	{#if loading && !detail}
		<p class="muted" role="status">Loading series…</p>
	{:else if detail}
		{#if loading}
			<p class="muted" role="status">Refreshing plan...</p>
		{/if}
		<div class="outcome-line">
			<StatusChip label={outcomeLabel(detail.outcome)} tone={outcomeTone(detail.outcome)} />
			{#if summary}
				<span class="muted">Usable history: {summary.h_length} weeks</span>
			{/if}
			{#if detail.horizon_start_week}
				<span class="muted">Horizon starts {detail.horizon_start_week}</span>
			{/if}
			{#if detail.imputation_method && (detail.imputed_weeks ?? 0) > 0}
				<span class="muted">
					Filled {detail.imputed_weeks} missing weeks ({imputationLabel(detail.imputation_method)})
				</span>
			{/if}
			{#if !detail.run_id}
				<span class="muted">No persisted forecast in the selected run.</span>
			{/if}
		</div>

		{#if detail.reason}
			<p class="banner banner-warn reason">Reason: {detail.reason}</p>
		{/if}

		<ForecastEditor {detail} {onSave} {onRefresh} {onDirtyChange} />

		<SeriesReviewPanel {detail} {onNote} {onReview} {onRefresh} />

		<SeriesChart seriesKey={seriesKey} history={detail.history} forecast={detail.forecast} planning={detail.planning} />

		<details class="panel model-details">
			<summary>Model details</summary>
			<dl class="meta">
				{#if summary}
					<div>
						<dt>Usable history</dt>
						<dd>{summary.h_length} weeks</dd>
					</div>
					<div>
						<dt>Qualification</dt>
						<dd>
							<StatusChip
								label={qualificationLabel(summary.qualification_status)}
								tone={qualificationTone(summary.qualification_status)}
							/>
						</dd>
					</div>
				{/if}
				{#if run}
					<div>
						<dt>Run</dt>
						<dd class="mono" title={run.id}>{shortId(run.id)}</dd>
					</div>
					<div>
						<dt>Run status</dt>
						<dd>
							<StatusChip label={humanizeToken(run.status)} tone={runTone(run.status)} />
						</dd>
					</div>
					<div>
						<dt>Horizon start</dt>
						<dd class="mono">{run.horizon_start_week}</dd>
					</div>
					<div>
						<dt>Engine</dt>
						<dd class="mono">{run.engine_version}</dd>
					</div>
					<div>
						<dt>Created</dt>
						<dd>{formatTimestamp(run.created_at)}</dd>
					</div>
				{/if}
				{#if detail.selected_model_id}
					<div>
						<dt>Selected model</dt>
						<dd class="mono">{detail.selected_model_id}</dd>
					</div>
				{/if}
				{#if detail.selection_score !== null}
					<div>
						<dt>Selection score</dt>
						<dd>{formatScore(detail.selection_score)}</dd>
					</div>
				{/if}
			</dl>

			{#if detail.run_id}
				<ModelComparison
					{models}
					loading={modelsLoading}
					error={modelsError}
					onRetry={onRetryModels}
				/>
			{/if}
		</details>
	{:else}
		<p class="muted">Select a series to inspect its history and forecast.</p>
	{/if}
</section>

<style>
	.detail {
		display: flex;
		flex-direction: column;
		gap: 0.9rem;
	}

	.head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 1rem;
	}

	.key {
		margin: 0 0 0.2rem;
		font-family: var(--font-mono);
		font-size: 1.15rem;
		overflow-wrap: anywhere;
	}

	.ids {
		margin: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 0.9rem;
		font-size: 0.8rem;
	}

	.meta {
		margin: 0;
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
		gap: 0.5rem 1rem;
		padding: 0.8rem 0.9rem;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		font-size: 0.88rem;
	}

	.meta dt {
		color: var(--ink-muted);
		font-weight: 600;
		font-size: 0.78rem;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	.meta dd {
		margin: 0.15rem 0 0;
		overflow-wrap: anywhere;
	}

	.outcome-line {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem;
		font-size: 0.88rem;
	}

	.model-details {
		padding: 0.8rem 0.9rem;
	}

	.model-details > summary {
		cursor: pointer;
		font-weight: 600;
		font-size: 0.92rem;
	}

	.model-details[open] > summary {
		margin-bottom: 0.7rem;
	}

	.model-details .meta {
		background: transparent;
		border: none;
		padding: 0;
	}

	.reason {
		margin: 0;
	}

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
		flex-shrink: 0;
	}
</style>
