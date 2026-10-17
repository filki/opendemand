<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import type {
		InventoryPreview,
		InventorySnapshot,
		PlanPublication,
		ReplenishmentRow
	} from '#lib/bindings.ts';
	import { formatQuantity, formatTimestamp } from '#lib/frontend/format.ts';
	import { t, type Locale } from '#lib/frontend/i18n.ts';
	import {
		exportReplenishmentXlsx,
		fetchInventorySnapshots,
		fetchPublicationsList,
		fetchReplenishmentPlan,
		inventoryImport,
		inventoryPreview,
		pickInventoryPolicy,
		pickInventoryReceipts,
		pickInventoryStock,
		pickInventoryWorkbook,
		pickXlsxExport,
		queryErrorCode
	} from '#lib/frontend/ipc.ts';

	let { locale, onClose }: { locale: Locale; onClose: () => void } = $props();

	const PAGE_SIZE = 50;

	let stockPath = $state<string | null>(null);
	let receiptsPath = $state<string | null>(null);
	let policyPath = $state<string | null>(null);
	let workbookPath = $state<string | null>(null);
	let unitConfirmed = $state(false);
	let receiptsEmptyConfirmed = $state(false);
	let preview = $state<InventoryPreview | null>(null);
	let previewBusy = $state(false);
	let importBusy = $state(false);
	let importError = $state<{ code: string; message: string } | null>(null);
	let importNotice = $state<string | null>(null);
	let selectedSnapshotId = $state<number | null>(null);
	let selectedPublicationId = $state<number | null>(null);
	let lowMultiplier = $state('');
	let highMultiplier = $state('');
	let searchInput = $state('');
	let debouncedSearch = $state('');
	let offset = $state(0);
	let resultScope = '';
	let exporting = $state(false);
	let exportError = $state<{ code: string; message: string } | null>(null);

	$effect(() => {
		const timer = setTimeout(() => {
			debouncedSearch = searchInput;
		}, 250);
		return () => clearTimeout(timer);
	});

	$effect(() => {
		const scope = `${selectedPublicationId}\u0000${selectedSnapshotId}\u0000${lowMultiplier}\u0000${highMultiplier}\u0000${debouncedSearch}`;
		if (scope !== resultScope) {
			resultScope = scope;
			offset = 0;
		}
	});

	const publicationsQuery = createQuery(() => ({
		queryKey: ['publications'],
		queryFn: fetchPublicationsList,
		retry: 0
	}));
	const publications: PlanPublication[] = $derived(publicationsQuery.data ?? []);
	$effect(() => {
		const first = publications[0];
		if (selectedPublicationId === null && first) selectedPublicationId = first.id;
	});

	const snapshotsQuery = createQuery(() => ({
		queryKey: ['inventory-snapshots'],
		queryFn: fetchInventorySnapshots,
		retry: 0
	}));
	const snapshots: InventorySnapshot[] = $derived(snapshotsQuery.data ?? []);
	$effect(() => {
		const first = snapshots[0];
		if (selectedSnapshotId === null && first) selectedSnapshotId = first.id;
	});

	const planQuery = createQuery(() => ({
		queryKey: [
			'replenishment',
			selectedPublicationId,
			selectedSnapshotId,
			lowMultiplier,
			highMultiplier,
			debouncedSearch,
			offset
		],
		queryFn: () =>
			fetchReplenishmentPlan(
				{
					publication_id: selectedPublicationId as number,
					snapshot_id: selectedSnapshotId as number,
					low_multiplier: lowMultiplier.trim() === '' ? null : lowMultiplier,
					high_multiplier: highMultiplier.trim() === '' ? null : highMultiplier
				},
				debouncedSearch,
				offset,
				PAGE_SIZE
			),
		enabled: selectedPublicationId !== null && selectedSnapshotId !== null,
		retry: 0
	}));
	const plan = $derived(planQuery.data ?? null);
	const rows = $derived(plan?.rows ?? []);
	const hasNext = $derived(rows.length === PAGE_SIZE);

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

	function currentEdit() {
		return {
			stock_path: stockPath,
			receipts_path: receiptsPath,
			policy_path: policyPath,
			workbook_path: workbookPath,
			receipts_empty_confirmed: receiptsEmptyConfirmed,
			unit_confirmed: unitConfirmed
		};
	}

	async function choose(kind: 'stock' | 'receipts' | 'policy' | 'workbook') {
		importError = null;
		preview = null;
		const path =
			kind === 'stock'
				? await pickInventoryStock()
				: kind === 'receipts'
					? await pickInventoryReceipts()
					: kind === 'policy'
						? await pickInventoryPolicy()
						: await pickInventoryWorkbook();
		if (path === null) return;
		if (kind === 'workbook') {
			workbookPath = path;
			stockPath = null;
			receiptsPath = null;
			policyPath = null;
		} else {
			workbookPath = null;
			if (kind === 'stock') stockPath = path;
			if (kind === 'receipts') receiptsPath = path;
			if (kind === 'policy') policyPath = path;
		}
	}

	function clearFiles() {
		stockPath = null;
		receiptsPath = null;
		policyPath = null;
		workbookPath = null;
		preview = null;
		importError = null;
	}

	async function runPreview() {
		if (previewBusy) return;
		previewBusy = true;
		importError = null;
		try {
			preview = await inventoryPreview(currentEdit());
		} catch (failure) {
			importError = classify(failure);
		} finally {
			previewBusy = false;
		}
	}

	async function runImport() {
		if (importBusy || preview === null || !preview.can_import) return;
		importBusy = true;
		importError = null;
		importNotice = null;
		try {
			const snapshot = await inventoryImport(currentEdit());
			importNotice = t(locale, 'inv.imported', {
				id: snapshot.id,
				week: snapshot.as_of_week,
				skus: snapshot.sku_count,
				receipts: snapshot.receipt_count,
				duplicate: snapshot.is_duplicate ? 'yes' : 'no'
			});
			selectedSnapshotId = snapshot.id;
			await snapshotsQuery.refetch();
		} catch (failure) {
			importError = classify(failure);
		} finally {
			importBusy = false;
		}
	}

	async function exportPlan() {
		if (exporting || selectedPublicationId === null || selectedSnapshotId === null) return;
		exporting = true;
		exportError = null;
		try {
			const path = await pickXlsxExport(`replenishment-${selectedPublicationId}`);
			if (!path) return;
			const result = await exportReplenishmentXlsx(
				{
					publication_id: selectedPublicationId,
					snapshot_id: selectedSnapshotId,
					low_multiplier: lowMultiplier.trim() === '' ? null : lowMultiplier,
					high_multiplier: highMultiplier.trim() === '' ? null : highMultiplier
				},
				path
			);
			importNotice = t(locale, 'repl.exported', { rows: result.row_count, path: result.path });
		} catch (failure) {
			exportError = classify(failure);
		} finally {
			exporting = false;
		}
	}

	function flagText(row: ReplenishmentRow): string {
		const parts = [
			...row.blocked.map((finding) => finding.code),
			...row.warnings.map((finding) => finding.code)
		];
		return parts.join(' · ');
	}
</script>

<section class="panel card replenishment" aria-labelledby="repl-heading">
	<header class="head">
		<h2 id="repl-heading">{t(locale, 'repl.heading')}</h2>
		<button class="btn btn-secondary" onclick={onClose}>{t(locale, 'fva.close')}</button>
	</header>

	<section class="inventory" aria-label={t(locale, 'inv.heading')}>
		<h3>{t(locale, 'inv.heading')}</h3>
		<p class="hint muted">{t(locale, 'inv.hint')}</p>
		<div class="file-row">
			<button class="btn btn-secondary btn-small" onclick={() => choose('workbook')}>
				{t(locale, 'inv.chooseWorkbook')}
			</button>
			<button class="btn btn-secondary btn-small" onclick={() => choose('stock')}>
				{t(locale, 'inv.chooseStock')}
			</button>
			<button class="btn btn-secondary btn-small" onclick={() => choose('receipts')}>
				{t(locale, 'inv.chooseReceipts')}
			</button>
			<button class="btn btn-secondary btn-small" onclick={() => choose('policy')}>
				{t(locale, 'inv.choosePolicy')}
			</button>
			<button class="btn btn-secondary btn-small" onclick={clearFiles}>{t(locale, 'inv.clearFiles')}</button>
		</div>
		<p class="muted small files">
			{#if workbookPath} workbook: <span class="mono">{workbookPath}</span>{/if}
			{#if stockPath} · stock: <span class="mono">{stockPath}</span>{/if}
			{#if receiptsPath} · receipts: <span class="mono">{receiptsPath}</span>{/if}
			{#if policyPath} · policy: <span class="mono">{policyPath}</span>{/if}
		</p>
		<label class="check">
			<input type="checkbox" bind:checked={unitConfirmed} />
			<span>{t(locale, 'inv.unitConfirmed')}</span>
		</label>
		<label class="check">
			<input type="checkbox" bind:checked={receiptsEmptyConfirmed} />
			<span>{t(locale, 'inv.receiptsEmpty')}</span>
		</label>
		<div class="file-row">
			<button class="btn btn-secondary btn-small" disabled={previewBusy} onclick={runPreview}>
				{previewBusy ? t(locale, 'inv.previewing') : t(locale, 'inv.preview')}
			</button>
			<button
				class="btn btn-primary btn-small"
				disabled={importBusy || preview === null || !preview.can_import}
				onclick={runImport}
			>
				{importBusy ? t(locale, 'inv.importing') : t(locale, 'inv.import')}
			</button>
		</div>
		{#if importError}
			<p class="banner banner-error small" role="alert">
				[{importError.code}] {importError.message}
			</p>
		{/if}
		{#if importNotice}
			<p class="banner banner-success small" role="status">{importNotice}</p>
		{/if}
		{#if preview}
			<p class="muted small">
				{t(locale, 'inv.snapshotLine', {
					id: '—',
					week: preview.as_of_week ?? '—',
					skus: preview.sku_count,
					receipts: preview.receipt_count
				})}
			</p>
			{#if preview.can_import}
				<p class="banner banner-success small">{t(locale, 'inv.canImport')}</p>
			{:else}
				<p class="banner banner-error small">
					{t(locale, 'inv.blockedImport', {
						count: preview.findings.filter((finding) => finding.severity === 'blocking').length
					})}
				</p>
			{/if}
			{#if preview.findings.length > 0}
				<ul class="findings">
					{#each preview.findings as finding, index (index)}
						<li class:blocking={finding.severity === 'blocking'}>
							<span class="chip {finding.severity === 'blocking' ? 'chip-error' : 'chip-warn'} tiny">
								{finding.code}
							</span>
							{finding.detail}
							{#if finding.row_ref}<span class="muted small"> ({finding.row_ref})</span>{/if}
						</li>
					{/each}
				</ul>
			{/if}
		{/if}
		<h4>{t(locale, 'inv.snapshots')}</h4>
		{#if snapshots.length === 0}
			<p class="muted small">{t(locale, 'inv.noSnapshots')}</p>
		{:else}
			<ul class="snapshots">
				{#each snapshots as snapshot (snapshot.id)}
					<li class:selected={selectedSnapshotId === snapshot.id}>
						<span>
							{t(locale, 'inv.snapshotLine', {
								id: snapshot.id,
								week: snapshot.as_of_week,
								skus: snapshot.sku_count,
								receipts: snapshot.receipt_count
							})}
							{#if snapshot.overdue_receipts > 0}
								<span class="chip chip-warn tiny">
									{t(locale, 'inv.overdue', { count: snapshot.overdue_receipts })}
								</span>
							{/if}
						</span>
						<button
							class="btn btn-secondary btn-small"
							onclick={() => (selectedSnapshotId = snapshot.id)}
						>
							{selectedSnapshotId === snapshot.id ? t(locale, 'inv.selected') : t(locale, 'inv.select')}
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<section class="plan" aria-label={t(locale, 'repl.heading')}>
		<div class="controls">
			<label>
				<span class="field-label">{t(locale, 'repl.publication')}</span>
				<select
					value={String(selectedPublicationId ?? '')}
					onchange={(event) => (selectedPublicationId = Number(event.currentTarget.value))}
				>
					{#each publications as publication (publication.id)}
						<option value={String(publication.id)}>
							#{publication.id} · {publication.cutoff_week} · {formatTimestamp(publication.published_at)}
						</option>
					{/each}
				</select>
			</label>
			<label>
				<span class="field-label">{t(locale, 'repl.snapshot')}</span>
				<select
					value={String(selectedSnapshotId ?? '')}
					onchange={(event) => (selectedSnapshotId = Number(event.currentTarget.value))}
				>
					{#each snapshots as snapshot (snapshot.id)}
						<option value={String(snapshot.id)}>#{snapshot.id} · {snapshot.as_of_week}</option>
					{/each}
				</select>
			</label>
			<label>
				<span class="field-label">{t(locale, 'repl.low')}</span>
				<input type="text" inputmode="decimal" class="mult-input" value={lowMultiplier} oninput={(event) => (lowMultiplier = event.currentTarget.value)} />
			</label>
			<label>
				<span class="field-label">{t(locale, 'repl.high')}</span>
				<input type="text" inputmode="decimal" class="mult-input" value={highMultiplier} oninput={(event) => (highMultiplier = event.currentTarget.value)} />
			</label>
			<label class="search">
				<span class="field-label">Search</span>
				<input
					type="search"
					placeholder={t(locale, 'repl.searchPlaceholder')}
					value={searchInput}
					oninput={(event) => (searchInput = event.currentTarget.value)}
				/>
			</label>
			<button
				class="btn btn-secondary btn-small"
				disabled={exporting || selectedPublicationId === null || selectedSnapshotId === null}
				onclick={exportPlan}
			>
				{t(locale, 'repl.export')}
			</button>
		</div>
		<p class="hint muted">
			{t(locale, 'repl.orderHint')} {t(locale, 'repl.scopeNote')}
		</p>
		{#if exportError}
			<p class="banner banner-error small" role="alert">
				[{exportError.code}] {exportError.message}
			</p>
		{/if}
		{#if selectedPublicationId === null || selectedSnapshotId === null}
			<p class="muted">{t(locale, 'repl.needSelection')}</p>
		{:else if planQuery.isPending}
			<p class="muted" role="status">{t(locale, 'repl.loading')}</p>
		{:else if planQuery.isError}
			<div class="banner banner-error" role="alert">
				<p>[{queryErrorCode(planQuery.error)}] {planQuery.error.message}</p>
				<button class="btn btn-secondary btn-small" onclick={() => void planQuery.refetch()}>
					{t(locale, 'common.retry')}
				</button>
			</div>
		{:else if plan}
			{#if rows.length === 0}
				<p class="muted">{t(locale, 'repl.noRows')}</p>
			{:else}
				<div class="table-wrap">
					<table>
						<thead>
							<tr>
								<th scope="col">{t(locale, 'repl.colSeries')}</th>
								<th scope="col">{t(locale, 'repl.colPolicy')}</th>
								<th scope="col">{t(locale, 'repl.colStock')}</th>
								<th scope="col">{t(locale, 'repl.colInbound')}</th>
								<th scope="col">{t(locale, 'repl.colDemand')}</th>
								<th scope="col">{t(locale, 'repl.colRaw')}</th>
								<th scope="col">{t(locale, 'repl.colOrder')}</th>
								<th scope="col">{t(locale, 'repl.colFlags')}</th>
							</tr>
						</thead>
						<tbody>
							{#each rows as row (row.product_id + ':' + row.location_id)}
								<tr>
									<th scope="row">
										{row.series_key}
										<span class="muted small"> · {row.location_code} · {row.unit}</span>
									</th>
									<td class="num">{row.lead_weeks} / {row.review_weeks} / {row.protection_weeks}</td>
									<td class="num">{formatQuantity(row.opening_stock)}</td>
									<td class="num">{formatQuantity(row.inbound_in_window)}</td>
									<td class="num">{formatQuantity(row.demand_window)}</td>
									<td class="num">{formatQuantity(row.raw_need)}</td>
									<td class="num order">
										{formatQuantity(row.order_base)}
										{#if row.order_low !== null || row.order_high !== null}
											<span class="muted small">
												(low {formatQuantity(row.order_low)} / high {formatQuantity(row.order_high)})
											</span>
										{/if}
									</td>
									<td class="flags">
										{#if row.blocked.length > 0}
											<span class="chip chip-error tiny">{t(locale, 'repl.blockedChip')}</span>
										{/if}
										{#if row.warnings.length > 0}
											<span class="chip chip-warn tiny">{t(locale, 'repl.warningChip')}</span>
										{/if}
										<span class="muted small">{flagText(row)}</span>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
				<div class="pager">
					<button
						class="btn btn-secondary btn-small"
						disabled={offset === 0 || planQuery.isFetching}
						onclick={() => (offset = Math.max(0, offset - PAGE_SIZE))}
					>
						{t(locale, 'common.previous')}
					</button>
					<span class="muted small">
						{t(locale, 'repl.range', { from: offset + 1, to: offset + rows.length, total: plan.total })}
					</span>
					<button
						class="btn btn-secondary btn-small"
						disabled={!hasNext || planQuery.isFetching}
						onclick={() => (offset = offset + PAGE_SIZE)}
					>
						{t(locale, 'common.next')}
					</button>
				</div>
			{/if}
		{/if}
	</section>
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

	.inventory {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		background: var(--surface-subtle);
		padding: 0.6rem 0.7rem;
		margin: 0.75rem 0;
	}

	.inventory h3,
	.inventory h4 {
		margin: 0 0 0.3rem;
	}

	.hint {
		margin: 0 0 0.4rem;
		font-size: 0.78rem;
	}

	.file-row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		margin: 0.35rem 0;
	}

	.files {
		overflow-wrap: anywhere;
	}

	.check {
		display: flex;
		gap: 0.4rem;
		align-items: flex-start;
		font-size: 0.85rem;
		margin: 0.2rem 0;
	}

	.findings {
		list-style: none;
		margin: 0.35rem 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		font-size: 0.82rem;
	}

	.snapshots {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
	}

	.snapshots li {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.3rem 0.5rem;
		font-size: 0.85rem;
	}

	.snapshots li.selected {
		border-color: var(--accent);
	}

	.controls {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
		align-items: flex-end;
		margin-bottom: 0.4rem;
	}

	.controls label {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	.mult-input {
		width: 5.5rem;
	}

	.search input {
		min-width: 14rem;
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
	}

	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.order {
		min-width: 8rem;
	}

	.flags {
		min-width: 12rem;
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
		font-size: 0.78rem;
	}
</style>
