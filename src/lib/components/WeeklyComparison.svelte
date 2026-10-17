<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import type { SeriesChangeSummary } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import WeeklyChangeDetail from '#lib/components/WeeklyChangeDetail.svelte';
	import { t, type Locale } from '#lib/frontend/i18n.ts';
	import {
		fetchWeeklyComparisonContext,
		fetchWeeklyComparisonDetail,
		fetchWeeklyComparisonList,
		queryErrorCode
	} from '#lib/frontend/ipc.ts';
	import {
		changedListEmptyText,
		WEEKLY_CHANGE_FILTERS,
		weeklyChangeFilterLabel,
		type WeeklyChangeFilter
	} from '#lib/frontend/weekly.ts';

	let { locale, onClose }: { locale: Locale; onClose: () => void } = $props();

	const PAGE_SIZE = 50;

	let searchInput = $state('');
	let debouncedSearch = $state('');
	let filter = $state<WeeklyChangeFilter>('all');
	let offset = $state(0);
	let selected = $state<{ productId: string; locationId: string } | null>(null);
	// The inputs a result page belongs to; a change resets page and selection
	// so a row filtered out of the new result can never stay selected.
	let resultScope = '';

	$effect(() => {
		const timer = setTimeout(() => {
			debouncedSearch = searchInput;
		}, 250);
		return () => clearTimeout(timer);
	});

	const contextQuery = createQuery(() => ({
		queryKey: ['weekly-context'],
		queryFn: fetchWeeklyComparisonContext,
		retry: 0
	}));
	const context = $derived(contextQuery.data ?? null);
	const currentRunId = $derived(context?.current?.id ?? null);
	const previousRunId = $derived(context?.previous?.id ?? null);

	$effect(() => {
		const scope = `${currentRunId ?? ''}\u0000${previousRunId ?? ''}\u0000${debouncedSearch}\u0000${filter}`;
		if (scope !== resultScope) {
			resultScope = scope;
			offset = 0;
			selected = null;
		}
	});

	const listQuery = createQuery(() => ({
		queryKey: ['weekly-list', currentRunId, previousRunId, debouncedSearch, filter, offset],
		queryFn: () =>
			fetchWeeklyComparisonList(
				currentRunId as string,
				previousRunId as string,
				debouncedSearch,
				filter,
				offset,
				PAGE_SIZE
			),
		enabled: currentRunId !== null && previousRunId !== null,
		retry: 0
	}));
	const changed = $derived(listQuery.data ?? []);
	const hasNext = $derived(changed.length === PAGE_SIZE);

	$effect(() => {
		if (selected === null && !listQuery.isFetching && listQuery.isFetched) {
			const first = (listQuery.data ?? [])[0];
			if (first) selected = { productId: first.product_id, locationId: first.location_id };
		}
	});

	const detailQuery = createQuery(() => ({
		queryKey: [
			'weekly-detail',
			currentRunId,
			previousRunId,
			selected?.productId ?? null,
			selected?.locationId ?? null
		],
		queryFn: () => {
			const selection = selected;
			if (selection === null) throw new Error('No series selected');
			return fetchWeeklyComparisonDetail(
				currentRunId as string,
				previousRunId as string,
				selection.productId,
				selection.locationId
			);
		},
		enabled: currentRunId !== null && previousRunId !== null && selected !== null,
		retry: 0
	}));

	function isSelected(item: SeriesChangeSummary): boolean {
		return (
			selected !== null &&
			selected.productId === item.product_id &&
			selected.locationId === item.location_id
		);
	}

	function handleSelect(item: SeriesChangeSummary) {
		selected = { productId: item.product_id, locationId: item.location_id };
	}
</script>

<section class="panel card comparison" aria-labelledby="comparison-heading">
	<header class="head">
		<div>
			<h2 id="comparison-heading">{t(locale, 'weekly.title')}</h2>
			{#if context && context.current && context.previous}
				<p class="muted small pair">
					{t(locale, 'weekly.pair', {
						previousRevision: context.previous.revision_id,
						currentRevision: context.current.revision_id
					})}
				</p>
			{/if}
		</div>
		<button class="btn btn-secondary" onclick={onClose}>{t(locale, 'weekly.close')}</button>
	</header>

	{#if contextQuery.isPending}
		<p class="muted" role="status">{t(locale, 'weekly.loading')}</p>
	{:else if contextQuery.isError}
		<div class="banner banner-error" role="alert">
			<p>[{queryErrorCode(contextQuery.error)}] {contextQuery.error.message}</p>
			<button class="btn btn-secondary" onclick={() => void contextQuery.refetch()}>
				{t(locale, 'common.retry')}
			</button>
		</div>
	{:else if !context || !context.current}
		<p class="muted">{t(locale, 'weekly.noCurrent')}</p>
	{:else if !context.previous}
		<p class="banner banner-warn">{t(locale, 'weekly.noPrevious')}</p>
	{:else}
		<div class="grid">
			<div class="list-pane">
				<label class="field-label" for="weekly-search">{t(locale, 'weekly.searchLabel')}</label>
				<input
					id="weekly-search"
					type="search"
					placeholder={t(locale, 'weekly.searchPlaceholder')}
					value={searchInput}
					oninput={(event) => (searchInput = event.currentTarget.value)}
				/>
				<label class="field-label" for="weekly-filter">{t(locale, 'weekly.filterLabel')}</label>
				<select
					id="weekly-filter"
					value={filter}
					onchange={(event) =>
						(filter = (event.currentTarget as HTMLSelectElement).value as WeeklyChangeFilter)}
				>
					{#each WEEKLY_CHANGE_FILTERS as option (option)}
						<option value={option}>{weeklyChangeFilterLabel(locale, option)}</option>
					{/each}
				</select>

				{#if listQuery.isError}
					<div class="banner banner-error" role="alert">
						<p>[{queryErrorCode(listQuery.error)}] {listQuery.error.message}</p>
						<button class="btn btn-secondary btn-small" onclick={() => void listQuery.refetch()}>
							{t(locale, 'common.retry')}
						</button>
					</div>
				{:else if listQuery.isPending && listQuery.isFetching}
					<p class="muted small" role="status">{t(locale, 'weekly.loadingList')}</p>
				{:else if changed.length === 0}
					<p class="muted small">
						{changedListEmptyText(locale, debouncedSearch, filter)}
					</p>
				{:else}
					<ul class="list" aria-label={t(locale, 'weekly.listAria')}>
						{#each changed as item (item.product_id + ':' + item.location_id)}
							<li>
								<button
									type="button"
									class="row"
									class:selected={isSelected(item)}
									aria-current={isSelected(item) ? 'true' : undefined}
									onclick={() => handleSelect(item)}
								>
									<span class="key mono" title={item.series_key}>{item.series_key}</span>
									<span class="row-meta">
										<span class="muted">{item.location_code}</span>
										{#if item.presence === 'current_only'}
											<StatusChip label={t(locale, 'weekly.presenceCurrentOnly')} tone="warn" />
										{:else if item.presence === 'previous_only'}
											<StatusChip label={t(locale, 'weekly.presencePreviousOnly')} tone="warn" />
										{/if}
										{#if item.demand_change_count > 0}
											<StatusChip
												label={t(locale, 'weekly.demandCount', { count: item.demand_change_count })}
												tone="neutral"
											/>
										{/if}
										{#if item.forecast_change_count > 0}
											<StatusChip
												label={t(locale, 'weekly.forecastCount', {
													count: item.forecast_change_count
												})}
												tone="neutral"
											/>
										{/if}
									</span>
								</button>
							</li>
						{/each}
					</ul>
				{/if}

				<div class="pager">
					<button
						class="btn btn-secondary btn-small"
						onclick={() => (offset = Math.max(0, offset - PAGE_SIZE))}
						disabled={offset === 0 || listQuery.isFetching}
					>
						{t(locale, 'common.previous')}
					</button>
					<span class="muted range">
						{changed.length > 0
							? t(locale, 'weekly.range', { from: offset + 1, to: offset + changed.length })
							: t(locale, 'weekly.nothingToShow')}
					</span>
					<button
						class="btn btn-secondary btn-small"
						onclick={() => (offset = offset + PAGE_SIZE)}
						disabled={!hasNext || listQuery.isFetching}
					>
						{t(locale, 'common.next')}
					</button>
				</div>
			</div>

			<div class="detail-pane">
				{#if selected === null}
					<p class="muted">{t(locale, 'weekly.selectHint')}</p>
				{:else}
					<WeeklyChangeDetail
						{locale}
						detail={detailQuery.data ?? null}
						loading={detailQuery.isPending}
						error={detailQuery.isError
							? { code: queryErrorCode(detailQuery.error), message: detailQuery.error.message }
							: null}
						onRetry={() => void detailQuery.refetch()}
					/>
				{/if}
			</div>
		</div>
	{/if}
</section>

<style>
	.comparison {
		padding: 1.2rem 1.4rem;
	}

	.head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 0.8rem;
		margin-bottom: 0.9rem;
	}

	h2 {
		margin: 0 0 0.25rem;
	}

	.small {
		font-size: 0.78rem;
	}

	.pair {
		margin: 0;
	}

	.grid {
		display: grid;
		grid-template-columns: minmax(16rem, 20rem) 1fr;
		gap: 1.2rem;
		align-items: start;
	}

	.list-pane input,
	.list-pane select {
		margin-bottom: 0.6rem;
		width: 100%;
	}

	.list {
		margin: 0;
		padding: 0;
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		max-height: 30rem;
		overflow-y: auto;
	}

	.row {
		width: 100%;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 0.2rem;
		padding: 0.45rem 0.55rem;
		border: 1px solid transparent;
		border-radius: var(--radius-sm);
		background: transparent;
		color: var(--ink);
		font: inherit;
		font-size: 0.88rem;
		text-align: left;
		cursor: pointer;
	}

	.row:hover {
		background: var(--surface-subtle);
		border-color: var(--border);
	}

	.row.selected {
		background: var(--cobalt-soft);
		border-color: rgba(32, 80, 200, 0.35);
	}

	.key {
		font-size: 0.85rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 100%;
	}

	.row-meta {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.35rem;
		font-size: 0.8rem;
	}

	.pager {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		margin-top: 0.7rem;
	}

	.range {
		font-size: 0.8rem;
	}

	.detail-pane {
		min-width: 0;
	}

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
	}

	@media (max-width: 1100px) {
		.grid {
			grid-template-columns: 1fr;
		}
	}
</style>
