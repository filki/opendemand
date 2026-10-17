<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import type { InboxItem } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import { formatQuantity, formatTimestamp } from '#lib/frontend/format.ts';
	import { t, type Locale, type MessageKey } from '#lib/frontend/i18n.ts';
	import {
		fetchExceptionInbox,
		queryErrorCode,
		saveReviewPolicy,
		setSeriesReview
	} from '#lib/frontend/ipc.ts';

	let {
		runId,
		locale,
		onOpenSeries,
		onClose
	}: {
		runId: string;
		locale: Locale;
		onOpenSeries: (productId: string, locationId: string) => void;
		onClose: () => void;
	} = $props();

	const PAGE_SIZE = 50;

	let searchInput = $state('');
	let debouncedSearch = $state('');
	let filter = $state<'all' | 'unreviewed' | 'reviewed'>('all');
	let offset = $state(0);
	let resultScope = '';
	let policyAbs = $state('');
	let policyRel = $state('');
	let biasHorizon = $state('');
	let biasWindow = $state('');
	let biasMin = $state('');
	let biasAbs = $state('');
	let policyBusy = $state(false);
	let policyError = $state<{ code: string; message: string } | null>(null);
	let seededPolicy = '';
	let rowBusy = $state<string | null>(null);
	let rowError = $state<{ key: string; message: string } | null>(null);

	const RULE_KEYS: Record<string, MessageKey> = {
		model_failure: 'inbox.rule.modelFailure',
		invalid_data: 'inbox.rule.invalidData',
		insufficient_history: 'inbox.rule.insufficientHistory',
		imputed_input: 'inbox.rule.imputedInput',
		production_fallback: 'inbox.rule.productionFallback',
		material_change: 'inbox.rule.materialChange',
		persistent_bias: 'inbox.rule.persistentBias',
		review_due: 'inbox.rule.reviewDue'
	};

	const RULE_TONES: Record<string, 'error' | 'warn' | 'info' | 'violet' | 'neutral'> = {
		model_failure: 'error',
		invalid_data: 'error',
		insufficient_history: 'warn',
		imputed_input: 'warn',
		production_fallback: 'warn',
		material_change: 'info',
		persistent_bias: 'info',
		review_due: 'violet'
	};

	$effect(() => {
		const timer = setTimeout(() => {
			debouncedSearch = searchInput;
		}, 250);
		return () => clearTimeout(timer);
	});

	// A new search or filter resets pagination so a page from an old result
	// set can never stay visible.
	$effect(() => {
		const scope = `${runId}\u0000${debouncedSearch}\u0000${filter}`;
		if (scope !== resultScope) {
			resultScope = scope;
			offset = 0;
			rowError = null;
		}
	});

	const inboxQuery = createQuery(() => ({
		queryKey: ['inbox', runId, debouncedSearch, filter, offset],
		queryFn: () => fetchExceptionInbox(runId, debouncedSearch, filter, offset, PAGE_SIZE),
		retry: 0
	}));
	const inbox = $derived(inboxQuery.data ?? null);
	const rows = $derived(inbox?.rows ?? []);
	const hasNext = $derived(rows.length === PAGE_SIZE);

	// Seed the policy inputs once per stored version; typing never reseeds.
	$effect(() => {
		const policy = inbox?.policy ?? null;
		if (!policy) return;
		const key = `${policy.version ?? 'none'}\u0000${policy.change_abs ?? ''}\u0000${policy.change_rel ?? ''}\u0000${policy.bias_horizon ?? ''}\u0000${policy.bias_window ?? ''}\u0000${policy.bias_min ?? ''}\u0000${policy.bias_abs ?? ''}`;
		if (key !== seededPolicy) {
			seededPolicy = key;
			policyAbs = policy.change_abs === null ? '' : String(policy.change_abs);
			policyRel = policy.change_rel === null ? '' : String(policy.change_rel);
			biasHorizon = policy.bias_horizon === null ? '' : String(policy.bias_horizon);
			biasWindow = policy.bias_window === null ? '' : String(policy.bias_window);
			biasMin = policy.bias_min === null ? '' : String(policy.bias_min);
			biasAbs = policy.bias_abs === null ? '' : String(policy.bias_abs);
		}
	});

	function ruleLabel(rule: string): string {
		const key = RULE_KEYS[rule];
		return key ? t(locale, key) : rule;
	}

	function ruleTone(rule: string): 'error' | 'warn' | 'info' | 'violet' | 'neutral' {
		return RULE_TONES[rule] ?? 'neutral';
	}

	function classify(failure: unknown): { code: string; message: string } {
		if (failure !== null && typeof failure === 'object') {
			const record = failure as { code?: unknown; message?: unknown };
			return {
				code: typeof record.code === 'string' && record.code !== '' ? record.code : 'unknown_error',
				message:
					typeof record.message === 'string' && record.message !== ''
						? record.message
						: 'The action could not be completed.'
			};
		}
		return { code: 'unknown_error', message: 'The action could not be completed.' };
	}

	async function applyPolicy(disable: boolean) {
		if (policyBusy) return;
		policyBusy = true;
		policyError = null;
		try {
			await saveReviewPolicy({
				change_abs: disable ? null : policyAbs,
				change_rel: disable ? null : policyRel,
				bias_horizon: disable ? null : biasHorizon,
				bias_window: disable ? null : biasWindow,
				bias_min: disable ? null : biasMin,
				bias_abs: disable ? null : biasAbs
			});
			await inboxQuery.refetch();
		} catch (failure) {
			policyError = classify(failure);
		} finally {
			policyBusy = false;
		}
	}

	function rowKey(item: InboxItem): string {
		return `${item.product_id}:${item.location_id}`;
	}

	async function act(item: InboxItem, action: 'reviewed' | 'reopened') {
		if (rowBusy !== null) return;
		rowBusy = rowKey(item);
		rowError = null;
		try {
			await setSeriesReview({
				run_id: runId,
				product_id: item.product_id,
				location_id: item.location_id,
				action
			});
			await inboxQuery.refetch();
		} catch (failure) {
			rowError = { key: rowKey(item), message: classify(failure).message };
		} finally {
			rowBusy = null;
		}
	}
</script>

<section class="panel card inbox" aria-labelledby="inbox-heading">
	<header class="head">
		<div>
			<h2 id="inbox-heading">{t(locale, 'inbox.title')}</h2>
			{#if inbox?.predecessor_run_id === null}
				<p class="muted small">{t(locale, 'inbox.predecessorMissing')}</p>
			{/if}
		</div>
		<button class="btn btn-secondary" onclick={onClose}>{t(locale, 'inbox.close')}</button>
	</header>

	{#if inboxQuery.isPending}
		<p class="muted" role="status">{t(locale, 'inbox.loading')}</p>
	{:else if inboxQuery.isError}
		<div class="banner banner-error" role="alert">
			<p>[{queryErrorCode(inboxQuery.error)}] {inboxQuery.error.message}</p>
			<button class="btn btn-secondary" onclick={() => void inboxQuery.refetch()}>
				{t(locale, 'common.retry')}
			</button>
		</div>
	{:else if inbox}
		<div class="controls">
			<div class="field">
				<label class="field-label" for="inbox-search">{t(locale, 'inbox.searchLabel')}</label>
				<input
					id="inbox-search"
					type="search"
					placeholder={t(locale, 'inbox.searchPlaceholder')}
					value={searchInput}
					oninput={(event) => (searchInput = event.currentTarget.value)}
				/>
			</div>
			<div class="field">
				<label class="field-label" for="inbox-filter">{t(locale, 'inbox.filterLabel')}</label>
				<select
					id="inbox-filter"
					value={filter}
					onchange={(event) =>
						(filter = (event.currentTarget as HTMLSelectElement).value as
							| 'all'
							| 'unreviewed'
							| 'reviewed')}
				>
					<option value="all">{t(locale, 'inbox.filter.all')}</option>
					<option value="unreviewed">{t(locale, 'inbox.filter.unreviewed')}</option>
					<option value="reviewed">{t(locale, 'inbox.filter.reviewed')}</option>
				</select>
			</div>
			<div class="policy">
				<div class="policy-head">
					<span class="field-label">{t(locale, 'inbox.policyHeading')}</span>
					{#if inbox.policy.version !== null}
						<span class="muted small">
							{t(locale, 'inbox.policyVersion', { version: inbox.policy.version })}
						</span>
					{/if}
				</div>
				<div class="policy-fields">
					<label>
						<span class="field-label">{t(locale, 'inbox.policyAbs')}</span>
						<input
							type="text"
							inputmode="decimal"
							class="policy-input"
							value={policyAbs}
							oninput={(event) => (policyAbs = event.currentTarget.value)}
						/>
					</label>
					<label>
						<span class="field-label">{t(locale, 'inbox.policyRel')}</span>
						<input
							type="text"
							inputmode="decimal"
							class="policy-input"
							value={policyRel}
							oninput={(event) => (policyRel = event.currentTarget.value)}
						/>
					</label>
					<div class="policy-actions">
						<button
							class="btn btn-primary btn-small"
							disabled={policyBusy || inbox.predecessor_run_id === null}
							onclick={() => applyPolicy(false)}
						>
							{policyBusy ? t(locale, 'inbox.policySaving') : t(locale, 'inbox.policySave')}
						</button>
						<button
							class="btn btn-secondary btn-small"
							disabled={policyBusy || inbox.policy.version === null}
							onclick={() => applyPolicy(true)}
						>
							{t(locale, 'inbox.policyDisable')}
						</button>
					</div>
				</div>
				<div class="policy-bias">
					<span class="field-label">{t(locale, 'inbox.policyBiasHeading')}</span>
					<div class="policy-fields">
						<label>
							<span class="field-label">{t(locale, 'inbox.policyBiasHorizon')}</span>
							<input
								type="text"
								inputmode="numeric"
								class="policy-input small-input"
								value={biasHorizon}
								oninput={(event) => (biasHorizon = event.currentTarget.value)}
							/>
						</label>
						<label>
							<span class="field-label">{t(locale, 'inbox.policyBiasWindow')}</span>
							<input
								type="text"
								inputmode="numeric"
								class="policy-input small-input"
								value={biasWindow}
								oninput={(event) => (biasWindow = event.currentTarget.value)}
							/>
						</label>
						<label>
							<span class="field-label">{t(locale, 'inbox.policyBiasMin')}</span>
							<input
								type="text"
								inputmode="numeric"
								class="policy-input small-input"
								value={biasMin}
								oninput={(event) => (biasMin = event.currentTarget.value)}
							/>
						</label>
						<label>
							<span class="field-label">{t(locale, 'inbox.policyBiasAbs')}</span>
							<input
								type="text"
								inputmode="decimal"
								class="policy-input small-input"
								value={biasAbs}
								oninput={(event) => (biasAbs = event.currentTarget.value)}
							/>
						</label>
					</div>
					<p class="muted small">{t(locale, 'inbox.policyBiasHint')}</p>
				</div>
				<p class="muted small">
					{inbox.policy.version === null ? t(locale, 'inbox.policyOff') : t(locale, 'inbox.policyHint')}
				</p>
				{#if policyError}
					<p class="banner banner-error small" role="alert">
						{t(locale, 'inbox.policyError', { message: `[${policyError.code}] ${policyError.message}` })}
					</p>
				{/if}
			</div>
		</div>

		{#if inboxQuery.isFetching && !inboxQuery.isPending}
			<p class="muted small" role="status">{t(locale, 'inbox.loading')}</p>
		{/if}

		{#if rows.length === 0}
			<p class="muted">{t(locale, 'inbox.noRows')}</p>
		{:else}
			<div class="table-wrap">
				<table class="rows" aria-label={t(locale, 'inbox.listAria')}>
					<thead>
						<tr>
							<th scope="col">{t(locale, 'weekly.searchLabel')}</th>
							<th scope="col">{t(locale, 'detail.change')}</th>
							<th scope="col">{t(locale, 'inbox.filterLabel')}</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as item (rowKey(item))}
							<tr>
								<td class="series-cell">
									<span class="key mono" title={item.series_key}>{item.series_key}</span>
									<span class="muted small location">{item.location_code}</span>
									<span class="chips">
										{#each item.rules as rule (rule)}
											<StatusChip label={ruleLabel(rule)} tone={ruleTone(rule)} />
										{/each}
									</span>
								</td>
								<td class="evidence-cell">
									{#if item.change_abs !== null}
										<span class="evidence">
											{t(locale, 'inbox.changeAbs', { value: formatQuantity(item.change_abs) })}
											{#if item.change_rel !== null}
												<span class="muted">({Math.round(item.change_rel * 100)}%)</span>
											{/if}
										</span>
									{/if}
									{#if item.bias_value !== null && item.bias_weeks !== null}
										<span class="evidence">
											{t(locale, 'inbox.biasChip', {
												direction: t(
													locale,
													item.bias_value > 0 ? 'inbox.biasOver' : 'inbox.biasUnder'
												),
												value: formatQuantity(Math.abs(item.bias_value) * 100, 1) + '%',
												weeks: item.bias_weeks
											})}
										</span>
									{/if}
									{#if item.due_review_date}
										<span class="evidence muted">
											{t(locale, 'inbox.reviewDueOn', { date: item.due_review_date })}
										</span>
									{/if}
									{#if item.note_count > 0}
										<span class="evidence muted">
											{t(locale, 'inbox.notes', { count: item.note_count })}
											{#if item.last_note_at}
												· {formatTimestamp(item.last_note_at)}
											{/if}
										</span>
									{/if}
									{#if item.reason}
										<span class="evidence muted">{item.reason}</span>
									{/if}
								</td>
								<td class="review-cell">
									{#if item.review_status === 'reviewed'}
										<StatusChip label={t(locale, 'inbox.statusReviewed')} tone="ok" />
										{#if item.reviewed_at}
											<span class="muted small">{formatTimestamp(item.reviewed_at)}</span>
										{/if}
										<button
											class="btn btn-secondary btn-small"
											disabled={rowBusy !== null}
											onclick={() => act(item, 'reopened')}
										>
											{t(locale, 'inbox.reopen')}
										</button>
									{:else}
										<StatusChip label={t(locale, 'inbox.statusUnreviewed')} tone="neutral" />
										<button
											class="btn btn-secondary btn-small"
											disabled={rowBusy !== null}
											onclick={() => act(item, 'reviewed')}
										>
											{t(locale, 'inbox.review')}
										</button>
									{/if}
									<button
										class="btn btn-primary btn-small"
										onclick={() => onOpenSeries(item.product_id, item.location_id)}
									>
										{t(locale, 'inbox.openSeries')}
									</button>
									{#if rowError && rowError.key === rowKey(item)}
										<span class="banner banner-error small" role="alert">
											{t(locale, 'inbox.errorAction', { message: rowError.message })}
										</span>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<div class="pager">
				<button
					class="btn btn-secondary btn-small"
					disabled={offset === 0 || inboxQuery.isFetching}
					onclick={() => (offset = Math.max(0, offset - PAGE_SIZE))}
				>
					{t(locale, 'common.previous')}
				</button>
				<span class="muted small">
					{t(locale, 'inbox.range', {
						from: offset + 1,
						to: offset + rows.length,
						total: inbox.total
					})}
				</span>
				<button
					class="btn btn-secondary btn-small"
					disabled={!hasNext || inboxQuery.isFetching}
					onclick={() => (offset = offset + PAGE_SIZE)}
				>
					{t(locale, 'common.next')}
				</button>
			</div>
		{/if}
	{/if}
</section>

<style>
	.head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 0.75rem;
	}

	.head h2 {
		margin: 0;
	}

	.controls {
		display: flex;
		flex-wrap: wrap;
		gap: 1rem;
		align-items: flex-start;
		margin: 0.75rem 0;
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
		min-width: 14rem;
	}

	.policy {
		flex: 1 1 24rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.6rem;
		background: var(--surface-subtle);
	}

	.policy-head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.policy-fields {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
		align-items: flex-end;
		margin: 0.35rem 0;
	}

	.policy-fields label {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	.policy-input {
		min-width: 8rem;
	}

	.small-input {
		min-width: 5rem;
	}

	.policy-bias {
		margin-top: 0.4rem;
		border-top: 1px dashed var(--border);
		padding-top: 0.4rem;
	}

	.policy-actions {
		display: flex;
		gap: 0.35rem;
	}

	.table-wrap {
		overflow-x: auto;
	}

	.rows {
		width: 100%;
		border-collapse: collapse;
	}

	.rows th {
		text-align: left;
		font-size: 0.78rem;
		text-transform: uppercase;
		letter-spacing: 0.03em;
		color: var(--muted);
		padding: 0.3rem 0.5rem;
		border-bottom: 1px solid var(--border);
	}

	.rows td {
		padding: 0.45rem 0.5rem;
		border-bottom: 1px solid var(--border);
		vertical-align: top;
	}

	.series-cell {
		min-width: 12rem;
	}

	.series-cell .location {
		display: block;
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 0.25rem;
		margin-top: 0.25rem;
	}

	.evidence-cell {
		min-width: 12rem;
	}

	.evidence {
		display: block;
		font-size: 0.84rem;
	}

	.review-cell {
		min-width: 10rem;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.35rem;
	}

	.pager {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.75rem;
		margin-top: 0.6rem;
	}

	.btn-small {
		min-height: 28px;
		padding: 0.2rem 0.55rem;
		font-size: 0.8rem;
	}

	.small {
		font-size: 0.8rem;
	}
</style>
