<script lang="ts">
	import type { RunSummary, SeriesDetail, SeriesSummary } from '$lib/bindings';
	import SeriesChart from '$lib/components/SeriesChart.svelte';
	import StatusChip from '$lib/components/StatusChip.svelte';
	import {
		formatScore,
		formatTimestamp,
		humanizeToken,
		outcomeLabel,
		qualificationLabel,
		shortId
	} from '$lib/frontend/format';

	let {
		summary,
		detail,
		run,
		loading,
		error,
		onRetry,
		onClear
	}: {
		summary: SeriesSummary | null;
		detail: SeriesDetail | null;
		run: RunSummary | null;
		loading: boolean;
		error: { code: string; message: string } | null;
		onRetry: () => void;
		onClear: () => void;
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
				<span title={locationId}>location {shortId(locationId, 13)}</span>
			</p>
		</div>
		<button class="btn btn-secondary btn-small" onclick={onClear}>Clear selection</button>
	</header>

	{#if error}
		<div class="banner banner-error" role="alert">
			<p>[{error.code}] {error.message}</p>
			<button class="btn btn-secondary btn-small" onclick={onRetry}>Retry</button>
		</div>
	{:else if loading && !detail}
		<p class="muted" role="status">Loading series…</p>
	{:else if detail}
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

		<div class="outcome-line">
			<StatusChip label={outcomeLabel(detail.outcome)} tone={outcomeTone(detail.outcome)} />
			{#if detail.horizon_start_week}
				<span class="muted">Horizon starts {detail.horizon_start_week}</span>
			{/if}
			{#if !detail.run_id}
				<span class="muted">No persisted forecast in the selected run.</span>
			{/if}
		</div>

		{#if detail.reason}
			<p class="banner banner-warn reason">Reason: {detail.reason}</p>
		{/if}

		<SeriesChart seriesKey={seriesKey} history={detail.history} forecast={detail.forecast} />
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
