<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import type { RunException } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import { formatPercent } from '#lib/frontend/format.ts';
	import { t, type Locale } from '#lib/frontend/i18n.ts';
	import { fetchRunOverview, queryErrorCode } from '#lib/frontend/ipc.ts';
	import { outcomeText } from '#lib/frontend/weekly.ts';

	let {
		runId,
		workspacePath,
		locale,
		selectionBlocked,
		onSelectSeries
	}: {
		runId: string;
		workspacePath: string;
		locale: Locale;
		/** True while a draft edit, save, or export must not be discarded. */
		selectionBlocked: boolean;
		onSelectSeries: (productId: string, locationId: string) => void;
	} = $props();

	const PAGE_SIZE = 20;
	/** All five persisted outcome keys; zero counts stay visible. */
	const OUTCOMES = ['ok', 'fallback_naive', 'insufficient_history', 'model_failure', 'invalid_data'];

	let offset = $state(0);
	// The workspace/run pair the current page belongs to; a change resets the
	// page so exceptions of an old run can never be shown.
	let resultScope = '';

	$effect(() => {
		const scope = `${workspacePath}\u0000${runId}`;
		if (scope !== resultScope) {
			resultScope = scope;
			offset = 0;
		}
	});

	const overviewQuery = createQuery(() => ({
		queryKey: ['run-overview', workspacePath, runId, offset],
		queryFn: () => fetchRunOverview(runId, offset, PAGE_SIZE),
		retry: 0
	}));
	const overview = $derived(overviewQuery.data ?? null);
	const hasNext = $derived(overview !== null && offset + PAGE_SIZE < overview.exception_count);

	function outcomeTone(outcome: string): 'ok' | 'info' | 'warn' | 'error' | 'neutral' {
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

	function countFor(outcome: string): number {
		return overview?.outcomes[outcome] ?? 0;
	}

	function handleException(item: RunException) {
		if (selectionBlocked) return;
		onSelectSeries(item.product_id, item.location_id);
	}
</script>

<section class="panel overview" aria-labelledby="overview-heading">
	{#if overviewQuery.isError}
		<div class="banner banner-error" role="alert">
			<p>[{queryErrorCode(overviewQuery.error)}] {overviewQuery.error.message}</p>
			<button class="btn btn-secondary btn-small" onclick={() => void overviewQuery.refetch()}>
				{t(locale, 'common.retry')}
			</button>
		</div>
	{:else if overview === null}
		<p class="muted small" role="status">{t(locale, 'overview.loading')}</p>
	{:else}
		<header class="head">
			<h2 id="overview-heading">{t(locale, 'overview.title')}</h2>
			{#if overview.series_count === 0}
				<p class="muted small">{t(locale, 'overview.noSeries')}</p>
			{:else}
				<p class="muted small">
					{t(locale, 'overview.coverage', {
						covered: overview.covered_series,
						total: overview.series_count,
						percent: formatPercent(overview.coverage)
					})}
					— {t(locale, 'overview.coverageNote')}
				</p>
			{/if}
		</header>

		<h3 class="sub">{t(locale, 'overview.outcomesHeading')}</h3>
		<div class="chips">
			{#each OUTCOMES as outcome (outcome)}
				<StatusChip
					label={`${outcomeText(locale, outcome)}: ${countFor(outcome)}`}
					tone={outcomeTone(outcome)}
				/>
			{/each}
		</div>

		<h3 class="sub">{t(locale, 'overview.exceptionsHeading', { count: overview.exception_count })}</h3>
		{#if overview.exception_count === 0}
			<p class="muted small">{t(locale, 'overview.noExceptions')}</p>
		{:else}
			<ul class="exceptions" aria-label={t(locale, 'overview.exceptionsHeading', { count: overview.exception_count })}>
				{#each overview.exceptions as item (item.product_id + ':' + item.location_id)}
					<li>
						<button
							type="button"
							class="exception"
							disabled={selectionBlocked}
							onclick={() => handleException(item)}
						>
							<span class="key mono" title={item.series_key}>{item.series_key}</span>
							<span class="muted location">{item.location_code}</span>
							<StatusChip label={outcomeText(locale, item.outcome)} tone={outcomeTone(item.outcome)} />
							{#if item.reason}
								<span class="reason muted">{item.reason}</span>
							{/if}
						</button>
					</li>
				{/each}
			</ul>
			<div class="pager">
				<button
					class="btn btn-secondary btn-small"
					onclick={() => (offset = Math.max(0, offset - PAGE_SIZE))}
					disabled={offset === 0 || overviewQuery.isFetching}
				>
					{t(locale, 'common.previous')}
				</button>
				<span class="muted range">
					{overview.exceptions.length > 0
						? t(locale, 'overview.range', {
								from: offset + 1,
								to: offset + overview.exceptions.length,
								count: overview.exception_count
							})
						: t(locale, 'overview.nothingToShow')}
				</span>
				<button
					class="btn btn-secondary btn-small"
					onclick={() => (offset = offset + PAGE_SIZE)}
					disabled={!hasNext || overviewQuery.isFetching}
				>
					{t(locale, 'common.next')}
				</button>
			</div>
		{/if}
	{/if}
</section>

<style>
	.overview {
		padding: 0.9rem 1.1rem;
	}

	.head {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		justify-content: space-between;
		gap: 0.4rem 1rem;
	}

	.head h2 {
		margin: 0;
		font-size: 0.95rem;
	}

	.head p {
		margin: 0;
		max-width: 72ch;
	}

	.sub {
		margin: 0.6rem 0 0.35rem;
		font-size: 0.8rem;
		color: var(--ink-muted);
		font-weight: 600;
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 0.35rem;
	}

	.exceptions {
		margin: 0;
		padding: 0;
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
		max-height: 22rem;
		overflow-y: auto;
	}

	.exception {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.35rem 0.5rem;
		border: 1px solid transparent;
		border-radius: var(--radius-sm);
		background: transparent;
		color: var(--ink);
		font: inherit;
		font-size: 0.84rem;
		text-align: left;
		cursor: pointer;
	}

	.exception:hover:not(:disabled) {
		background: var(--surface-subtle);
		border-color: var(--border);
	}

	.exception:disabled {
		cursor: not-allowed;
		opacity: 0.7;
	}

	.key {
		font-size: 0.82rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 20rem;
	}

	.location {
		flex-shrink: 0;
	}

	.reason {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.pager {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		margin-top: 0.5rem;
	}

	.range {
		font-size: 0.8rem;
	}

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
	}
</style>