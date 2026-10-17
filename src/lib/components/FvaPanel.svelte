<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import type { FvaPoint, PublicationFva } from '#lib/bindings.ts';
	import { formatQuantity, formatTimestamp } from '#lib/frontend/format.ts';
	import { t, type Locale } from '#lib/frontend/i18n.ts';
	import {
		fetchActualsConfirmationLatest,
		fetchFvaPublicationSeries,
		fetchFvaReport,
		queryErrorCode,
		setActualsConfirmation
	} from '#lib/frontend/ipc.ts';

	let {
		locale,
		latestRevisionId,
		onClose
	}: {
		locale: Locale;
		latestRevisionId: number | null;
		onClose: () => void;
	} = $props();

	const PAGE_SIZE = 50;
	const HORIZONS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

	let horizon = $state(2);
	let searchInput = $state('');
	let debouncedSearch = $state('');
	let offset = $state(0);
	let resultScope = '';
	let inspectPublication = $state<number | null>(null);
	let inspectOffset = $state(0);
	let confirmationRevision = $state('');
	let confirmationThrough = $state('');
	let confirmationBusy = $state(false);
	let confirmationError = $state<{ code: string; message: string } | null>(null);
	let confirmationNotice = $state<string | null>(null);

	$effect(() => {
		const timer = setTimeout(() => {
			debouncedSearch = searchInput;
		}, 250);
		return () => clearTimeout(timer);
	});

	$effect(() => {
		const scope = `${horizon}\u0000${debouncedSearch}`;
		if (scope !== resultScope) {
			resultScope = scope;
			offset = 0;
			inspectOffset = 0;
		}
	});

	$effect(() => {
		if (confirmationRevision === '' && latestRevisionId !== null) {
			confirmationRevision = String(latestRevisionId);
		}
	});

	const confirmationQuery = createQuery(() => ({
		queryKey: ['actuals-confirmation'],
		queryFn: fetchActualsConfirmationLatest,
		retry: 0
	}));
	const confirmation = $derived(confirmationQuery.data ?? null);

	const reportQuery = createQuery(() => ({
		queryKey: ['fva-report', horizon, debouncedSearch, offset],
		queryFn: () => fetchFvaReport(horizon, debouncedSearch, offset, PAGE_SIZE),
		retry: 0
	}));
	const report = $derived(reportQuery.data ?? null);
	const rows = $derived(report?.rows ?? []);
	const hasNext = $derived(rows.length === PAGE_SIZE);

	const pointsQuery = createQuery(() => ({
		queryKey: ['fva-points', inspectPublication, horizon, debouncedSearch, inspectOffset],
		queryFn: () =>
			fetchFvaPublicationSeries(
				inspectPublication as number,
				horizon,
				debouncedSearch,
				inspectOffset,
				PAGE_SIZE
			),
		enabled: inspectPublication !== null,
		retry: 0
	}));
	const points = $derived(pointsQuery.data ?? null);
	const pointsHasNext = $derived((points?.rows.length ?? 0) === PAGE_SIZE);

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

	async function saveConfirmation() {
		if (confirmationBusy) return;
		const revision = Number(confirmationRevision);
		if (!Number.isInteger(revision) || revision <= 0 || confirmationThrough === '') {
			confirmationError = {
				code: 'invalid_input',
				message: 'Choose the actuals revision and the finished week it is complete through.'
			};
			return;
		}
		confirmationBusy = true;
		confirmationError = null;
		confirmationNotice = null;
		try {
			const saved = await setActualsConfirmation(revision, confirmationThrough);
			confirmationNotice = t(locale, 'fva.confirmationCurrent', {
				week: saved.confirmed_through,
				revision: saved.revision_id,
				time: formatTimestamp(saved.created_at)
			});
			confirmationThrough = '';
			await confirmationQuery.refetch();
			await reportQuery.refetch();
		} catch (failure) {
			confirmationError = classify(failure);
		} finally {
			confirmationBusy = false;
		}
	}

	function percent(value: number | null): string {
		return value === null ? '—' : `${(value * 100).toFixed(1)}%`;
	}

	function pointsLabel(value: number | null): string {
		return value === null ? '—' : `${formatQuantity(value * 100, 1)} pp`;
	}

	function toggleInspect(publication: PublicationFva) {
		inspectOffset = 0;
		inspectPublication = inspectPublication === publication.publication_id ? null : publication.publication_id;
	}

	function pointExcluded(point: FvaPoint): string {
		return point.excluded ?? '—';
	}
</script>

<section class="panel card performance" aria-labelledby="fva-heading">
	<header class="head">
		<h2 id="fva-heading">{t(locale, 'fva.title')}</h2>
		<button class="btn btn-secondary" onclick={onClose}>{t(locale, 'fva.close')}</button>
	</header>
	<p class="hint muted">{t(locale, 'fva.notes')}</p>

	{#if reportQuery.isPending}
		<p class="muted" role="status">{t(locale, 'fva.loading')}</p>
	{:else if reportQuery.isError}
		<div class="banner banner-error" role="alert">
			<p>[{queryErrorCode(reportQuery.error)}] {reportQuery.error.message}</p>
			<button class="btn btn-secondary" onclick={() => void reportQuery.refetch()}>
				{t(locale, 'common.retry')}
			</button>
		</div>
	{:else if report}
		<section class="confirmation" aria-label={t(locale, 'fva.confirmationHeading')}>
			<span class="field-label">{t(locale, 'fva.confirmationHeading')}</span>
			{#if confirmation}
				<p class="muted small">
					{t(locale, 'fva.confirmationCurrent', {
						week: confirmation.confirmed_through,
						revision: confirmation.revision_id,
						time: formatTimestamp(confirmation.created_at)
					})}
				</p>
			{:else}
				<p class="banner banner-warn small">{t(locale, 'fva.confirmationNone')}</p>
			{/if}
			<p class="hint muted">{t(locale, 'fva.confirmationHint')}</p>
			<div class="confirmation-form">
				<label>
					<span class="field-label">{t(locale, 'fva.confirmationRevision')}</span>
					<input
						type="number"
						min="1"
						class="confirmation-input"
						value={confirmationRevision}
						oninput={(event) => (confirmationRevision = event.currentTarget.value)}
					/>
				</label>
				<label>
					<span class="field-label">{t(locale, 'fva.confirmationThrough')}</span>
					<input
						type="date"
						class="confirmation-input"
						value={confirmationThrough}
						oninput={(event) => (confirmationThrough = event.currentTarget.value)}
					/>
				</label>
				<button class="btn btn-primary btn-small" disabled={confirmationBusy} onclick={saveConfirmation}>
					{confirmationBusy ? t(locale, 'fva.confirmationSaving') : t(locale, 'fva.confirmationSave')}
				</button>
			</div>
			{#if confirmationError}
				<p class="banner banner-error small" role="alert">
					[{confirmationError.code}] {confirmationError.message}
				</p>
			{/if}
			{#if confirmationNotice}
				<p class="banner banner-success small" role="status">{confirmationNotice}</p>
			{/if}
		</section>

		<div class="controls">
			<label>
				<span class="field-label">{t(locale, 'fva.horizonLabel')}</span>
				<select
					value={String(horizon)}
					onchange={(event) => (horizon = Number((event.currentTarget as HTMLSelectElement).value))}
				>
					{#each HORIZONS as step (step)}
						<option value={String(step)}>h={step}</option>
					{/each}
				</select>
			</label>
			<label class="search">
				<span class="field-label">{t(locale, 'fva.searchLabel')}</span>
				<input
					type="search"
					placeholder={t(locale, 'fva.searchPlaceholder')}
					value={searchInput}
					oninput={(event) => (searchInput = event.currentTarget.value)}
				/>
			</label>
			<span class="muted small horizon-hint">{t(locale, 'fva.horizonHint')}</span>
		</div>

		<h3 class="section-heading">{t(locale, 'fva.publicationsHeading')}</h3>
		{#if report.publications.length === 0}
			<p class="muted">{t(locale, 'fva.awaitingConfirmation')}</p>
		{:else}
			<div class="publication-list">
				{#each report.publications as publication (publication.publication_id)}
					<article class="publication">
						<div class="publication-head">
							<span class="publication-title">
								{t(locale, 'fva.publicationTitle', {
									id: publication.publication_id,
									cutoff: publication.cutoff_week
								})}
							</span>
							{#if confirmation === null}
								<span class="muted small">{t(locale, 'fva.awaitingConfirmation')}</span>
							{/if}
						</div>
						<p class="muted small counts">
							{t(locale, 'fva.publicationCounts', {
								scored: publication.scored,
								adjusted: publication.adjusted,
								improved: publication.improved,
								harmful: publication.harmful,
								unchanged: publication.unchanged,
								imputed: publication.imputed_input
							})}
						</p>
						<p class="muted small exclusions">
							{t(locale, 'fva.exclusions', {
								future: publication.excluded.future,
								late: publication.excluded.late_published,
								hindsight: publication.excluded.hindsight,
								absent: publication.excluded.absent,
								missing: publication.excluded.missing_or_partial,
								stockout: publication.excluded.stockout,
								noForecast: publication.excluded.no_forecast
							})}
						</p>
						{#if publication.low_sample}
							<p class="banner banner-warn small">{t(locale, 'fva.lowSample')}</p>
						{/if}
						<button
							class="btn btn-secondary btn-small"
							disabled={confirmation === null}
							onclick={() => toggleInspect(publication)}
						>
							{inspectPublication === publication.publication_id
								? t(locale, 'fva.inspectClose')
								: t(locale, 'fva.inspect')}
						</button>
						{#if inspectPublication === publication.publication_id}
							<div class="points">
								{#if pointsQuery.isPending}
									<p class="muted small" role="status">{t(locale, 'fva.pointsLoading')}</p>
								{:else if pointsQuery.isError}
									<div class="banner banner-error small" role="alert">
										<p>[{queryErrorCode(pointsQuery.error)}] {pointsQuery.error.message}</p>
									</div>
								{:else if points}
									{#if points.rows.length === 0}
										<p class="muted small">{t(locale, 'fva.pointsEmpty')}</p>
									{:else}
										<div class="table-wrap">
											<table aria-label={t(locale, 'fva.seriesTableAria')}>
												<thead>
													<tr>
														<th scope="col">{t(locale, 'fva.colSeries')}</th>
														<th scope="col">{t(locale, 'fva.colWeek')}</th>
														<th scope="col">{t(locale, 'fva.colActual')}</th>
														<th scope="col">{t(locale, 'fva.colBaseline')}</th>
														<th scope="col">{t(locale, 'fva.colEffective')}</th>
														<th scope="col">{t(locale, 'fva.colBaseErr')}</th>
														<th scope="col">{t(locale, 'fva.colEffErr')}</th>
														<th scope="col">{t(locale, 'fva.colAdjustment')}</th>
														<th scope="col">{t(locale, 'fva.colExcluded')}</th>
													</tr>
												</thead>
												<tbody>
													{#each points.rows as point (point.product_id + ':' + point.location_id)}
														<tr>
															<th scope="row">
																{point.series_key}
																<span class="muted small"> · {point.location_code}</span>
																{#if point.imputed_input}
																	<span class="chip chip-warn tiny">{t(locale, 'fva.imputedFlag')}</span>
																{/if}
															</th>
															<td class="mono">{point.week_start}</td>
															<td class="num">{formatQuantity(point.actual)}</td>
															<td class="num">{formatQuantity(point.baseline)}</td>
															<td class="num">{formatQuantity(point.effective)}</td>
															<td class="num">{formatQuantity(point.baseline_error)}</td>
															<td class="num">{formatQuantity(point.effective_error)}</td>
															<td class="num">{formatQuantity(point.adjustment)}</td>
															<td>
																{#if point.excluded}
																	<span class="chip chip-warn tiny">{pointExcluded(point)}</span>
																{:else}
																	<span class="chip chip-ok tiny">scored</span>
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
												disabled={inspectOffset === 0 || pointsQuery.isFetching}
												onclick={() => (inspectOffset = Math.max(0, inspectOffset - PAGE_SIZE))}
											>
												{t(locale, 'common.previous')}
											</button>
											<span class="muted small">{t(locale, 'fva.pointsTotal', { total: points.total })}</span>
											<button
												class="btn btn-secondary btn-small"
												disabled={!pointsHasNext || pointsQuery.isFetching}
												onclick={() => (inspectOffset = inspectOffset + PAGE_SIZE)}
											>
												{t(locale, 'common.next')}
											</button>
										</div>
									{/if}
								{/if}
							</div>
						{/if}
					</article>
				{/each}
			</div>
		{/if}

		<h3 class="section-heading">{t(locale, 'fva.seriesLabel', {})}</h3>
		{#if rows.length === 0}
			<p class="muted">{t(locale, 'fva.noSeries')}</p>
		{:else}
			<div class="table-wrap">
				<table aria-label={t(locale, 'fva.seriesTableAria')}>
					<thead>
						<tr>
							<th scope="col">{t(locale, 'fva.colSeries')}</th>
							<th scope="col">{t(locale, 'fva.publicationScoredLabel', {})}</th>
							<th scope="col">WAPE base</th>
							<th scope="col">WAPE eff</th>
							<th scope="col">Δ WAPE</th>
							<th scope="col">MAE base</th>
							<th scope="col">MAE eff</th>
							<th scope="col">Δ MAE</th>
							<th scope="col">Bias base</th>
							<th scope="col">Bias eff</th>
						</tr>
					</thead>
					<tbody>
						{#each rows as row (row.product_id + ':' + row.location_id)}
							<tr>
								<th scope="row">
									{row.series_key}
									<span class="muted small"> · {row.location_code}</span>
									{#if row.imputed_input}
										<span class="chip chip-warn tiny">{t(locale, 'fva.imputedFlag')}</span>
									{/if}
								</th>
								<td class="num">
									{row.scored}
									{#if row.adjusted > 0}
										<span class="muted small"> · adj {row.adjusted}</span>
									{/if}
								</td>
								<td class="num">{percent(row.measures.baseline_wape)}</td>
								<td class="num">{percent(row.measures.effective_wape)}</td>
								<td class="num">{pointsLabel(row.measures.wape_improvement_pp)}</td>
								<td class="num">{formatQuantity(row.measures.baseline_mae)}</td>
								<td class="num">{formatQuantity(row.measures.effective_mae)}</td>
								<td class="num">{formatQuantity(row.measures.mae_improvement)}</td>
								<td class="num">{percent(row.measures.baseline_bias)}</td>
								<td class="num">{percent(row.measures.effective_bias)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<div class="pager">
				<button
					class="btn btn-secondary btn-small"
					disabled={offset === 0 || reportQuery.isFetching}
					onclick={() => (offset = Math.max(0, offset - PAGE_SIZE))}
				>
					{t(locale, 'common.previous')}
				</button>
				<span class="muted small">
					{t(locale, 'fva.range', { from: offset + 1, to: offset + rows.length, total: report.total })}
				</span>
				<button
					class="btn btn-secondary btn-small"
					disabled={!hasNext || reportQuery.isFetching}
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

	.hint {
		margin: 0.25rem 0 0.5rem;
		font-size: 0.8rem;
	}

	.confirmation {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		background: var(--surface-subtle);
		padding: 0.6rem 0.7rem;
		margin-bottom: 0.75rem;
	}

	.confirmation-form {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
		align-items: flex-end;
		margin-top: 0.35rem;
	}

	.confirmation-form label {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	.confirmation-input {
		min-width: 10rem;
	}

	.controls {
		display: flex;
		flex-wrap: wrap;
		gap: 1rem;
		align-items: flex-end;
		margin-bottom: 0.5rem;
	}

	.controls label {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	.search input {
		min-width: 16rem;
	}

	.horizon-hint {
		max-width: 28rem;
	}

	.section-heading {
		margin: 0.75rem 0 0.35rem;
	}

	.publication-list {
		display: flex;
		flex-direction: column;
		gap: 0.6rem;
	}

	.publication {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.55rem 0.7rem;
	}

	.publication-title {
		font-weight: 600;
	}

	.counts,
	.exclusions {
		margin: 0.25rem 0;
	}

	.points {
		margin-top: 0.5rem;
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
		padding: 0.3rem 0.45rem;
		border-bottom: 1px solid var(--border);
		font-size: 0.84rem;
		white-space: nowrap;
	}

	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.pager {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.75rem;
		margin-top: 0.5rem;
	}

	.btn-small {
		min-height: 28px;
		padding: 0.2rem 0.55rem;
		font-size: 0.8rem;
	}

	.tiny {
		font-size: 0.7rem;
		padding: 0.05rem 0.35rem;
	}

	.small {
		font-size: 0.8rem;
	}
</style>
