<script lang="ts">
	import type { SeriesComparisonDetail } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import { formatQuantity, humanizeToken } from '#lib/frontend/format.ts';
	import { t, type Locale } from '#lib/frontend/i18n.ts';
	import {
		changeKindLabel,
		forecastPointNote,
		formatDelta,
		observationStatusText,
		outcomeText,
		planSourceText,
		presenceLabel
	} from '#lib/frontend/weekly.ts';

	let {
		locale,
		detail,
		loading,
		error,
		onRetry
	}: {
		locale: Locale;
		detail: SeriesComparisonDetail | null;
		loading: boolean;
		error: { code: string; message: string } | null;
		onRetry: () => void;
	} = $props();

	function quantityCell(quantity: number | null, status: string | null): string {
		if (quantity === null && status === null) return '—';
		return `${formatQuantity(quantity)}${status ? ` (${observationStatusText(locale, status)})` : ''}`;
	}
</script>

{#if error}
	<div class="banner banner-error" role="alert">
		<p>[{error.code}] {error.message}</p>
		<button class="btn btn-secondary btn-small" onclick={onRetry}>{t(locale, 'common.retry')}</button>
	</div>
{:else if loading}
	<p class="muted" role="status">{t(locale, 'detail.loading')}</p>
{:else if detail}
	<article class="detail" aria-label={t(locale, 'detail.aria')}>
		<header class="detail-head">
			<div>
				<h3 class="key mono">{detail.series_key}</h3>
				<p class="muted small">{detail.location_code} · {detail.location_id}</p>
			</div>
			<div class="chips">
				{#if detail.presence !== 'both'}
					<StatusChip
						label={presenceLabel(locale, detail.presence) ?? humanizeToken(detail.presence)}
						tone="warn"
					/>
				{/if}
				<StatusChip
					label={t(locale, 'detail.outcomeNew', { outcome: outcomeText(locale, detail.current_outcome) })}
					tone="neutral"
				/>
				<StatusChip
					label={t(locale, 'detail.outcomePrevious', {
						outcome: outcomeText(locale, detail.previous_outcome)
					})}
					tone="neutral"
				/>
			</div>
		</header>

		<section class="block" aria-labelledby="demand-heading">
			<h4 id="demand-heading">{t(locale, 'detail.demandHeading', { count: detail.demand_changes.length })}</h4>
			{#if detail.demand_changes.length === 0}
				<p class="muted small">{t(locale, 'detail.demandEmpty')}</p>
			{:else}
				<table>
					<thead>
						<tr>
							<th scope="col">{t(locale, 'detail.week')}</th>
							<th scope="col">{t(locale, 'detail.previous')}</th>
							<th scope="col">{t(locale, 'detail.current')}</th>
							<th scope="col">{t(locale, 'detail.change')}</th>
						</tr>
					</thead>
					<tbody>
						{#each detail.demand_changes as point (point.week_start)}
							<tr>
								<td class="mono">{point.week_start}</td>
								<td>{quantityCell(point.previous_quantity, point.previous_status)}</td>
								<td>{quantityCell(point.current_quantity, point.current_status)}</td>
								<td>{changeKindLabel(locale, point.change_kind)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>

		<section class="block" aria-labelledby="forecast-heading">
			<h4 id="forecast-heading">
				{t(locale, 'detail.forecastHeading', {
					count: detail.forecast_changes.filter((point) => point.is_change).length
				})}
			</h4>
			{#if detail.forecast_changes.length === 0}
				<p class="muted small">{t(locale, 'detail.forecastEmpty')}</p>
			{:else}
				<table>
					<thead>
						<tr>
							<th scope="col">{t(locale, 'detail.week')}</th>
							<th scope="col">{t(locale, 'detail.previous')}</th>
							<th scope="col">{t(locale, 'detail.current')}</th>
							<th scope="col">{t(locale, 'detail.delta')}</th>
							<th scope="col">{t(locale, 'detail.note')}</th>
						</tr>
					</thead>
					<tbody>
						{#each detail.forecast_changes as point (point.week_start)}
							<tr>
								<td class="mono">{point.week_start}</td>
								<td>{formatQuantity(point.previous_value)}</td>
								<td>{formatQuantity(point.current_value)}</td>
								<td>{formatDelta(point.delta)}</td>
								<td class="muted">{forecastPointNote(locale, point) || '—'}</td>
							</tr>
						{/each}
					</tbody>
				</table>
				<p class="muted small">{t(locale, 'detail.deltaNote')}</p>
			{/if}
		</section>

		<section class="block" aria-labelledby="plan-heading">
			<h4 id="plan-heading">{t(locale, 'detail.planHeading')}</h4>
			{#if detail.planning.length === 0}
				<p class="muted small">{t(locale, 'detail.planEmpty')}</p>
			{:else}
				<table>
					<thead>
						<tr>
							<th scope="col">{t(locale, 'detail.week')}</th>
							<th scope="col">{t(locale, 'detail.baseline')}</th>
							<th scope="col">{t(locale, 'detail.override')}</th>
							<th scope="col">{t(locale, 'detail.effective')}</th>
							<th scope="col">{t(locale, 'detail.source')}</th>
						</tr>
					</thead>
					<tbody>
						{#each detail.planning as point (point.week_start)}
							<tr>
								<td class="mono">{point.week_start}</td>
								<td>{formatQuantity(point.baseline)}</td>
								<td>{formatQuantity(point.override_value)}</td>
								<td>{formatQuantity(point.effective)}</td>
								<td class="muted">{planSourceText(locale, point.value_source)}</td>
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
