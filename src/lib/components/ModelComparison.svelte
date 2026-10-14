<script lang="ts">
	import type { SeriesModelComparison } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import { formatQuantity, formatScore } from '#lib/frontend/format.ts';

	let {
		models,
		loading,
		error,
		onRetry
	}: {
		models: SeriesModelComparison[] | null;
		loading: boolean;
		error: { code: string; message: string } | null;
		onRetry: () => void;
	} = $props();

	type Tone = 'neutral' | 'ok' | 'warn' | 'error' | 'info' | 'violet';

	function stateLabel(model: SeriesModelComparison): string {
		if (model.production_status === 'failed') return 'Failed';
		switch (model.eligibility_status) {
			case 'not_eligible':
				return 'Not eligible';
			case 'production_only':
				return 'Production only';
			default:
				return model.origins_completed >= 2 ? 'Evaluated candidate' : 'Production only';
		}
	}

	function stateTone(model: SeriesModelComparison): Tone {
		if (model.production_status === 'failed') return 'error';
		if (model.is_champion) return 'ok';
		switch (model.eligibility_status) {
			case 'not_eligible':
				return 'neutral';
			case 'production_only':
				return 'info';
			default:
				return 'violet';
		}
	}

	function roleLabel(model: SeriesModelComparison): string {
		if (model.is_champion) return model.selected ? 'Champion · used' : 'Champion · not used';
		if (model.selected) return 'Fallback · used';
		return '—';
	}

	function metricLabel(model: SeriesModelComparison): string {
		if (model.comparable_metric === 'wape') return 'WAPE';
		if (model.comparable_metric === 'mae') return 'MAE';
		return '—';
	}

	function formatBias(value: number | null): string {
		if (value === null || !Number.isFinite(value)) return '—';
		const formatted = formatScore(value);
		return value > 0 ? `+${formatted}` : formatted;
	}

	function note(model: SeriesModelComparison): string {
		if (model.production_status === 'failed' && model.failure_code) {
			return `Production failed: ${model.failure_code}`;
		}
		if (model.eligibility_status === 'not_eligible') return 'Below the model training floor';
		if (model.eligibility_status === 'production_only') {
			return 'Needs at least two completed backtest origins';
		}
		if (model.is_champion) return 'Best comparable backtest accuracy';
		if (model.selected) return 'Used after a higher-ranked model failed';
		return 'Higher comparable score than the champion';
	}

	const hasOrigins = $derived((models ?? []).some((model) => model.origins.length > 0));
</script>

<section class="comparison" aria-labelledby="model-comparison-heading">
	<h3 id="model-comparison-heading">Model comparison</h3>
	{#if error}
		<div class="banner banner-error" role="alert">
			<p>[{error.code}] {error.message}</p>
			<button class="btn btn-secondary btn-small" onclick={onRetry}>Retry</button>
		</div>
	{:else if loading && !models}
		<p class="muted" role="status">Loading model comparison…</p>
	{:else if models && models.length > 0}
		<div class="table-wrap">
			<table>
				<caption class="muted">
					Per-series comparison across candidate models for this run. Scores compare models only on
					backtest origins all candidates completed; lower is better.
				</caption>
				<thead>
					<tr>
						<th scope="col">Model</th>
						<th scope="col">State</th>
						<th scope="col">Origins</th>
						<th scope="col">Score</th>
						<th scope="col">WAPE</th>
						<th scope="col">MAE</th>
						<th scope="col">Bias</th>
						<th scope="col">Role</th>
					</tr>
				</thead>
				<tbody>
					{#each models as model (model.model_id)}
						<tr class:selected-row={model.selected}>
							<th scope="row" class="mono">{model.model_id}</th>
							<td><StatusChip label={stateLabel(model)} tone={stateTone(model)} /></td>
							<td class="num">{model.origins_completed}/{model.origins_planned}</td>
							<td class="num">{model.selection_score === null ? '—' : `${formatScore(model.selection_score)} ${metricLabel(model)}`}</td>
							<td class="num">{formatScore(model.wape)}</td>
							<td class="num">{formatQuantity(model.mae)}</td>
							<td class="num" title="Positive means overforecasting; unavailable when the actual total is zero">
								{formatBias(model.bias)}
							</td>
							<td>{roleLabel(model)}</td>
						</tr>
						<tr class="note-row" class:selected-row={model.selected}>
							<td colspan="8" class="note">{note(model)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		{#if hasOrigins}
			<details class="origins">
				<summary>Backtest origin evidence</summary>
				<table>
					<thead>
						<tr>
							<th scope="col">Model</th>
							<th scope="col">Origin</th>
							<th scope="col">Train</th>
							<th scope="col">Status</th>
							<th scope="col">WAPE</th>
							<th scope="col">MAE</th>
							<th scope="col">Failure</th>
						</tr>
					</thead>
					<tbody>
						{#each models as model (model.model_id)}
							{#each model.origins as origin (origin.origin_index)}
								<tr>
									<th scope="row" class="mono">{model.model_id}</th>
									<td class="mono">{origin.origin_week_start}</td>
									<td class="num">{origin.train_size}w</td>
									<td>{origin.status}</td>
									<td class="num">{formatScore(origin.wape)}</td>
									<td class="num">{formatQuantity(origin.mae)}</td>
									<td>{origin.failure_code ?? '—'}</td>
								</tr>
							{/each}
						{/each}
					</tbody>
				</table>
			</details>
		{/if}
	{:else}
		<p class="muted">No stored model evaluation for this series in the selected run.</p>
	{/if}
</section>

<style>
	.comparison {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	h3 {
		margin: 0;
		font-size: 0.98rem;
	}

	.table-wrap {
		overflow-x: auto;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--surface);
	}

	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.84rem;
	}

	caption {
		text-align: left;
		padding: 0.5rem 0.75rem 0;
		font-size: 0.76rem;
	}

	th,
	td {
		padding: 0.35rem 0.75rem;
		text-align: left;
		vertical-align: middle;
	}

	thead th {
		border-bottom: 1px solid var(--border);
		color: var(--ink-muted);
		font-size: 0.74rem;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	tbody th {
		font-weight: 600;
		overflow-wrap: anywhere;
	}

	.num {
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}

	.note-row td {
		padding-top: 0;
		color: var(--ink-muted);
		font-size: 0.78rem;
	}

	.selected-row {
		background: rgba(46, 86, 155, 0.06);
	}

	.origins summary {
		cursor: pointer;
		font-size: 0.84rem;
		color: var(--ink-muted);
	}

	.origins table {
		margin-top: 0.4rem;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--surface);
	}

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
		flex-shrink: 0;
	}
</style>
