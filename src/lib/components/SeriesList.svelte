<script lang="ts">
	import type { SeriesSummary } from '$lib/bindings';
	import StatusChip from '$lib/components/StatusChip.svelte';
	import { qualificationLabel } from '$lib/frontend/format';

	let {
		series,
		loading,
		disabled = false,
		error,
		selected,
		search,
		offset,
		pageSize,
		onSearch,
		onSelect,
		onPrev,
		onNext,
		onRetry
	}: {
		series: SeriesSummary[];
		loading: boolean;
		disabled?: boolean;
		error: { code: string; message: string } | null;
		selected: { productId: string; locationId: string } | null;
		search: string;
		offset: number;
		pageSize: number;
		onSearch: (value: string) => void;
		onSelect: (item: SeriesSummary) => void;
		onPrev: () => void;
		onNext: () => void;
		onRetry: () => void;
	} = $props();

	const hasNext = $derived(series.length === pageSize);

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

	function isSelected(item: SeriesSummary): boolean {
		return (
			selected !== null &&
			selected.productId === item.product_id &&
			selected.locationId === item.location_id
		);
	}
</script>

<section class="panel section" aria-labelledby="series-heading">
	<h2 id="series-heading">Series</h2>

	<label class="field-label" for="series-search">Search series</label>
	<input
		id="series-search"
		type="search"
		placeholder="Filter by series key"
		value={search}
		oninput={(event) => onSearch(event.currentTarget.value)}
	/>

	{#if error}
		<div class="banner banner-error" role="alert">
			<p>[{error.code}] {error.message}</p>
			<button class="btn btn-secondary btn-small" onclick={onRetry}>Retry</button>
		</div>
	{:else if loading}
		<p class="muted status" role="status">Loading series…</p>
	{:else if series.length === 0}
		<p class="muted status">
			{search ? 'No series match this search.' : 'No series in this revision yet.'}
		</p>
	{:else}
		<ul class="list" aria-label="Series in this revision">
			{#each series as item (item.product_id + ':' + item.location_id)}
				<li>
					<button
						type="button"
						class="row"
						class:selected={isSelected(item)}
						aria-current={isSelected(item) ? 'true' : undefined}
						{disabled}
						onclick={() => onSelect(item)}
					>
						<span class="key" title={item.series_key}>{item.series_key}</span>
						<span class="row-meta">
							<span class="muted" title={item.location_id}>{item.location_code}</span>
							<span class="muted">{item.h_length} w history</span>
							<StatusChip
								label={qualificationLabel(item.qualification_status)}
								tone={qualificationTone(item.qualification_status)}
							/>
						</span>
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	<div class="pager">
		<button class="btn btn-secondary btn-small" onclick={onPrev} disabled={offset === 0 || loading}>
			Previous
		</button>
		<span class="muted range">
			{series.length > 0 ? `Showing ${offset + 1}–${offset + series.length}` : 'Nothing to show'}
		</span>
		<button class="btn btn-secondary btn-small" onclick={onNext} disabled={!hasNext || loading}>
			Next
		</button>
	</div>
</section>

<style>
	.section {
		padding: 1rem 1.1rem;
	}

	h2 {
		margin-bottom: 0.6rem;
	}

	input {
		margin-bottom: 0.6rem;
	}

	.status {
		font-size: 0.88rem;
	}

	.list {
		margin: 0;
		padding: 0;
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		max-height: 22rem;
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
		font-family: var(--font-mono);
		font-size: 0.85rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 100%;
	}

	.row-meta {
		display: flex;
		align-items: center;
		gap: 0.45rem;
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

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
	}
</style>
