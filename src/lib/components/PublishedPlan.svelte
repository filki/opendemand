<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import type { PlanPublication, SeriesDetail } from '#lib/bindings.ts';
	import { formatQuantity, formatTimestamp, overrideReasonLabel } from '#lib/frontend/format.ts';
	import { t, type Locale } from '#lib/frontend/i18n.ts';
	import { fetchPublicationSeriesDetail, queryErrorCode } from '#lib/frontend/ipc.ts';

	let {
		detail,
		publication,
		locale
	}: {
		detail: SeriesDetail;
		publication: PlanPublication;
		locale: Locale;
	} = $props();

	const publishedQuery = createQuery(() => ({
		queryKey: ['publication-series', publication.id, detail.product_id, detail.location_id],
		queryFn: () =>
			fetchPublicationSeriesDetail(publication.id, detail.product_id, detail.location_id),
		retry: 0
	}));
	const published = $derived(publishedQuery.data ?? null);
</script>

<section class="published" aria-labelledby="published-heading">
	<h3 id="published-heading">
		{t(locale, 'publication.viewTitle', {
			id: publication.id,
			time: formatTimestamp(publication.published_at)
		})}
	</h3>
	<p class="hint muted">{t(locale, 'publication.viewHint')}</p>

	{#if publishedQuery.isPending}
		<p class="muted" role="status">{t(locale, 'publication.loading')}</p>
	{:else if publishedQuery.isError}
		<div class="banner banner-error" role="alert">
			<p>[{queryErrorCode(publishedQuery.error)}] {publishedQuery.error.message}</p>
			<button class="btn btn-secondary" onclick={() => void publishedQuery.refetch()}>
				{t(locale, 'common.retry')}
			</button>
		</div>
	{:else if published}
		<div class="table-wrap">
			<table>
				<thead>
					<tr>
						<th scope="col">{t(locale, 'publication.colWeek')}</th>
						<th scope="col">{t(locale, 'publication.colBaseline')}</th>
						<th scope="col">{t(locale, 'publication.colOverride')}</th>
						<th scope="col">{t(locale, 'publication.colEffective')}</th>
						<th scope="col">{t(locale, 'publication.colSource')}</th>
					</tr>
				</thead>
				<tbody>
					{#each published.planning as point (point.week_start)}
						<tr>
							<th scope="row" class="mono">{point.week_start}</th>
							<td class="num">{formatQuantity(point.baseline)}</td>
							<td class="num">
								{#if point.override_value === null}
									<span class="muted">—</span>
								{:else}
									{formatQuantity(point.override_value)}
									{#if point.override_reason}
										<span class="muted small"> · {overrideReasonLabel(point.override_reason)}</span>
									{/if}
								{/if}
							</td>
							<td class="num">
								{#if point.effective === null}
									<span class="muted">—</span>
								{:else}
									{formatQuantity(point.effective)}
								{/if}
							</td>
							<td>
								<span class="chip chip-neutral">{point.value_source}</span>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</section>

<style>
	.published h3 {
		margin: 0 0 0.25rem;
	}

	.hint {
		margin: 0 0 0.5rem;
		font-size: 0.8rem;
	}

	.table-wrap {
		overflow-x: auto;
	}

	table {
		width: 100%;
		border-collapse: collapse;
	}

	th,
	td {
		text-align: left;
		padding: 0.3rem 0.5rem;
		border-bottom: 1px solid var(--border);
		font-size: 0.86rem;
	}

	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.small {
		font-size: 0.78rem;
	}
</style>
