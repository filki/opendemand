<script lang="ts">
	import type { SeriesComparisonDetail } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import { planValueSourceLabel } from '#lib/frontend/chart.ts';
	import { formatQuantity, humanizeToken, observationStatusLabel, outcomeLabel } from '#lib/frontend/format.ts';
	import { forecastPointNote, formatDelta, presenceLabel } from '#lib/frontend/weekly.ts';

	let {
		detail,
		loading,
		error,
		onRetry
	}: {
		detail: SeriesComparisonDetail | null;
		loading: boolean;
		error: { code: string; message: string } | null;
		onRetry: () => void;
	} = $props();

	function demandChangeLabel(kind: string): string {
		switch (kind) {
			case 'added':
				return 'Added';
			case 'removed':
				return 'Removed';
			case 'changed':
				return 'Changed';
			default:
				return humanizeToken(kind);
		}
	}

	function quantityCell(quantity: number | null, status: string | null): string {
		if (quantity === null && status === null) return '—';
		return `${formatQuantity(quantity)}${status ? ` (${observationStatusLabel(status)})` : ''}`;
	}
</script>

{#if error}
	<div class="banner banner-error" role="alert">
		<p>[{error.code}] {error.message}</p>
		<button class="btn btn-secondary btn-small" onclick={onRetry}>Retry</button>
	</div>
{:else if loading}
	<p class="muted" role="status">Loading series comparison…</p>
{:else if detail}
	<article class="detail" aria-label="Series comparison detail">
		<header class="detail-head">
			<div>
				<h3 class="key mono">{detail.series_key}</h3>
				<p class="muted small">{detail.location_code} · {detail.location_id}</p>
			</div>
			<div class="chips">
				{#if detail.presence !== 'both'}
					<StatusChip
						label={presenceLabel(detail.presence) ?? humanizeToken(detail.presence)}
						tone="warn"
					/>
				{/if}
				<StatusChip label={`New run: ${outcomeLabel(detail.current_outcome)}`} tone="neutral" />
				<StatusChip label={`Previous run: ${outcomeLabel(detail.previous_outcome)}`} tone="neutral" />
			</div>
		</header>

		<section class="block" aria-labelledby="demand-heading">
			<h4 id="demand-heading">History changes ({detail.demand_changes.length})</h4>
			{#if detail.demand_changes.length === 0}
				<p class="muted small">No observation cell changed between the two revisions.</p>
			{:else}
				<table>
					<thead>
						<tr>
							<th scope="col">Week</th>
							<th scope="col">Previous</th>
							<th scope="col">Current</th>
							<th scope="col">Change</th>
						</tr>
					</thead>
					<tbody>
						{#each detail.demand_changes as point (point.week_start)}
							<tr>
								<td class="mono">{point.week_start}</td>
								<td>{quantityCell(point.previous_quantity, point.previous_status)}</td>
								<td>{quantityCell(point.current_quantity, point.current_status)}</td>
								<td>{demandChangeLabel(point.change_kind)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>

		<section class="block" aria-labelledby="forecast-heading">
			<h4 id="forecast-heading">Baseline forecast changes ({detail.forecast_changes.filter((point) => point.is_change).length})</h4>
			{#if detail.forecast_changes.length === 0}
				<p class="muted small">No baseline forecast exists in either compared run.</p>
			{:else}
				<table>
					<thead>
						<tr>
							<th scope="col">Week</th>
							<th scope="col">Previous</th>
							<th scope="col">Current</th>
							<th scope="col">Delta</th>
							<th scope="col">Note</th>
						</tr>
					</thead>
					<tbody>
						{#each detail.forecast_changes as point (point.week_start)}
							<tr>
								<td class="mono">{point.week_start}</td>
								<td>{formatQuantity(point.previous_value)}</td>
								<td>{formatQuantity(point.current_value)}</td>
								<td>{formatDelta(point.delta)}</td>
								<td class="muted">{forecastPointNote(point) || '—'}</td>
							</tr>
						{/each}
					</tbody>
				</table>
				<p class="muted small">Deltas compare immutable baselines on common calendar weeks only. Overrides never carry between runs.</p>
			{/if}
		</section>

		<section class="block" aria-labelledby="plan-heading">
			<h4 id="plan-heading">Current plan (latest run)</h4>
			{#if detail.planning.length === 0}
				<p class="muted small">The current run has no planning rows for this series.</p>
			{:else}
				<table>
					<thead>
						<tr>
							<th scope="col">Week</th>
							<th scope="col">Baseline</th>
							<th scope="col">Override</th>
							<th scope="col">Effective</th>
							<th scope="col">Source</th>
						</tr>
					</thead>
					<tbody>
						{#each detail.planning as point (point.week_start)}
							<tr>
								<td class="mono">{point.week_start}</td>
								<td>{formatQuantity(point.baseline)}</td>
								<td>{formatQuantity(point.override_value)}</td>
								<td>{formatQuantity(point.effective)}</td>
								<td class="muted">{planValueSourceLabel(point.value_source)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>
	</article>
{/if}

<style>
	.detail {
		display: flex;
		flex-direction: column;
		gap: 0.9rem;
		min-width: 0;
	}

	.detail-head {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-start;
		justify-content: space-between;
		gap: 0.6rem;
	}

	.detail-head h3 {
		margin: 0;
		font-size: 0.95rem;
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 0.35rem;
	}

	.block h4 {
		margin: 0 0 0.4rem;
		font-size: 0.85rem;
	}

	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.82rem;
	}

	th,
	td {
		padding: 0.3rem 0.45rem;
		text-align: left;
		border-bottom: 1px solid var(--border);
		white-space: nowrap;
	}

	th {
		color: var(--ink-muted);
		font-weight: 600;
	}

	tbody tr:last-child td {
		border-bottom: none;
	}

	.small {
		font-size: 0.78rem;
	}

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
	}
</style>
