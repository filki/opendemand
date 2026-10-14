<script lang="ts">
	import type { ParsingDescriptor } from '#lib/bindings.ts';
	import {
		adoptDetectionOnce,
		blockingFindings,
		canImport,
		DATE_FORMAT_CHOICES,
		descriptorFromForm,
		IMPORT_DELIMITERS,
		IMPORT_ENCODINGS,
		emptyForm,
		mappingComplete,
		sourceFormat,
		warningFindings,
		type WizardForm
	} from '#lib/frontend/import-wizard.ts';
	import { describeError, pickImportSource, previewImport } from '#lib/frontend/ipc.ts';
	import { t, type Locale } from '#lib/frontend/i18n.ts';

	let {
		locale,
		onImport,
		onClose
	}: {
		locale: Locale;
		onImport: (path: string, descriptor: ParsingDescriptor) => Promise<void>;
		onClose?: () => void;
	} = $props();

	let path = $state<string | null>(null);
	let preview = $state<Awaited<ReturnType<typeof previewImport>> | null>(null);
	let form = $state<WizardForm>(emptyForm());
	let busy = $state(false);
	let importing = $state(false);
	let error = $state<string | null>(null);
	let adoptDetection = $state(false);
	let requestId = 0;

	const blocking = $derived(preview ? blockingFindings(preview.findings) : []);
	const warnings = $derived(preview ? warningFindings(preview.findings) : []);
	const ready = $derived(canImport(preview, form, busy || importing));

	async function chooseFile() {
		error = null;
		try {
			const selected = await pickImportSource();
			if (!selected) return;
			path = selected;
			preview = null;
			form = emptyForm();
			adoptDetection = true;
			await refresh();
		} catch (caught) {
			error = describeError(caught).message;
		}
	}

	async function refresh() {
		if (path === null) return;
		const token = ++requestId;
		busy = true;
		error = null;
		const current = preview;
		const format = current ? sourceFormat(current) : 'csv';
		const descriptor = mappingComplete(form)
			? descriptorFromForm(form, format, current?.sheets ?? [])
			: null;
		try {
			const result = await previewImport(path, descriptor);
			if (token !== requestId) return;
			preview = result;
			if (descriptor === null) {
				const outcome = adoptDetectionOnce(form, result, adoptDetection);
				form = outcome.form;
				if (outcome.applied) adoptDetection = false;
			}
		} catch (caught) {
			if (token !== requestId) return;
			error = describeError(caught).message;
		} finally {
			if (token === requestId) busy = false;
		}
	}

	function toggleKeyColumn(header: string) {
		const selected = new Set(form.keyColumns);
		if (selected.has(header)) selected.delete(header);
		else selected.add(header);
		// Declared order is header order; the backend composes in this order.
		form.keyColumns = (preview?.headers ?? []).filter((candidate) => selected.has(candidate));
		void refresh();
	}

	function toggleDateFormat(format: string) {
		const formats = new Set(form.dateFormats);
		if (formats.has(format)) formats.delete(format);
		else formats.add(format);
		form.dateFormats = DATE_FORMAT_CHOICES.filter((choice) => formats.has(choice));
		void refresh();
	}

	async function runImport() {
		if (path === null || preview === null || !ready) return;
		importing = true;
		error = null;
		try {
			await onImport(path, descriptorFromForm(form, sourceFormat(preview), preview.sheets));
		} catch (caught) {
			error = describeError(caught).message;
		} finally {
			importing = false;
		}
	}
</script>

<section class="panel card" aria-labelledby="wizard-heading" aria-busy={busy}>
	<div class="head">
		<h2 id="wizard-heading">{t(locale, 'wizard.title')}</h2>
		{#if onClose}
			<button class="btn btn-secondary" onclick={onClose}>{t(locale, 'wizard.cancel')}</button>
		{/if}
	</div>
	<p class="muted">{t(locale, 'wizard.intro')}</p>

	<div class="row">
		<button class="btn btn-primary" onclick={chooseFile}>
			{path === null ? t(locale, 'wizard.chooseFile') : t(locale, 'wizard.changeFile')}
		</button>
		{#if path !== null}
			<span class="mono muted">{path}</span>
		{:else}
			<span class="muted">{t(locale, 'wizard.pickHint')}</span>
		{/if}
	</div>

	{#if busy}
		<p class="muted" role="status">{t(locale, 'wizard.refreshing')}</p>
	{/if}
	{#if error}
		<div class="banner banner-error" role="alert"><p>{error}</p></div>
	{/if}

	{#if preview !== null}
		<div class="grid">
			<fieldset>
				<legend>{t(locale, 'wizard.structure')}</legend>
				{#if preview.sheets.length > 0}
					<label for="wizard-sheet">{t(locale, 'wizard.sheet')}</label>
					<select id="wizard-sheet" bind:value={form.sheet} onchange={() => void refresh()}>
						{#each preview.sheets as sheet (sheet)}
							<option value={sheet}>{sheet}</option>
						{/each}
					</select>
				{:else}
					<label for="wizard-encoding">{t(locale, 'wizard.encoding')}</label>
					<select id="wizard-encoding" bind:value={form.encoding} onchange={() => void refresh()}>
						{#each IMPORT_ENCODINGS as encoding (encoding)}
							<option value={encoding}>{encoding}</option>
						{/each}
					</select>
					<label for="wizard-delimiter">{t(locale, 'wizard.delimiter')}</label>
					<select
						id="wizard-delimiter"
						bind:value={form.delimiter}
						onchange={() => void refresh()}
					>
						{#each IMPORT_DELIMITERS as delimiter (delimiter)}
							<option value={delimiter}>{delimiter === '\t' ? 'tab' : delimiter}</option>
						{/each}
					</select>
				{/if}
				<label for="wizard-header-row">{t(locale, 'wizard.headerRow')}</label>
				<input
					id="wizard-header-row"
					type="number"
					min="1"
					bind:value={form.headerRow}
					onchange={() => void refresh()}
				/>
			</fieldset>

			<fieldset>
				<legend>{t(locale, 'wizard.columns')}</legend>
				<p class="muted small">{t(locale, 'wizard.keyColumnsHint')}</p>
				<span class="label">{t(locale, 'wizard.keyColumns')}</span>
				<div class="keys">
					{#each preview.headers as header, headerIndex (header)}
						<label class="check">
							<input
								type="checkbox"
								checked={form.keyColumns.includes(header)}
								onchange={() => toggleKeyColumn(header)}
							/>
							<span>{header || `#${headerIndex + 1}`}</span>
						</label>
					{/each}
				</div>
				<label for="wizard-key-separator">{t(locale, 'wizard.keySeparator')}</label>
				<input id="wizard-key-separator" type="text" maxlength="3" bind:value={form.keySeparator} />
				<label for="wizard-date-column">{t(locale, 'wizard.dateColumn')}</label>
				<select
					id="wizard-date-column"
					bind:value={form.dateColumn}
					onchange={() => void refresh()}
				>
					<option value="">—</option>
					{#each preview.headers as header (header)}
						<option value={header}>{header}</option>
					{/each}
				</select>
				<span class="label">{t(locale, 'wizard.dateFormats')}</span>
				<div class="keys">
					{#each DATE_FORMAT_CHOICES as choice (choice)}
						<label class="check">
							<input
								type="checkbox"
								checked={form.dateFormats.includes(choice)}
								onchange={() => toggleDateFormat(choice)}
							/>
							<span class="mono">{choice}</span>
						</label>
					{/each}
				</div>
				<label for="wizard-quantity-column">{t(locale, 'wizard.quantityColumn')}</label>
				<select
					id="wizard-quantity-column"
					bind:value={form.quantityColumn}
					onchange={() => void refresh()}
				>
					<option value="">—</option>
					{#each preview.headers as header (header)}
						<option value={header}>{header}</option>
					{/each}
				</select>
				<label for="wizard-decimal">{t(locale, 'wizard.decimalSeparator')}</label>
				<select id="wizard-decimal" bind:value={form.decimalSeparator} onchange={() => void refresh()}>
					<option value=".">.</option>
					<option value=",">,</option>
				</select>
				<label for="wizard-thousands">{t(locale, 'wizard.thousandsSeparator')}</label>
				<select
					id="wizard-thousands"
					bind:value={form.thousandsSeparator}
					onchange={() => void refresh()}
				>
					<option value="">{t(locale, 'wizard.none')}</option>
					<option value=".">.</option>
					<option value=",">,</option>
					<option value=" ">␠</option>
				</select>
			</fieldset>
		</div>

		{#if mappingComplete(form)}
			<h3>{t(locale, 'wizard.sample')}</h3>
			<div class="sample">
				<table>
					<thead>
						<tr>
							<th>{t(locale, 'wizard.seriesKey')}</th>
							<th>{t(locale, 'wizard.weekStart')}</th>
							<th>{t(locale, 'wizard.quantity')}</th>
							<th>{t(locale, 'wizard.status')}</th>
						</tr>
					</thead>
					<tbody>
						{#each preview.parsed_rows as row, index (index)}
							<tr>
								<td class="mono">{row.series_key}</td>
								<td class="mono">{row.week_start}</td>
								<td class="mono">{row.quantity}</td>
								<td>{row.status}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{:else}
			<p class="muted">{t(locale, 'wizard.mappingIncomplete')}</p>
		{/if}

		<h3>{t(locale, 'wizard.findings')}</h3>
		{#if blocking.length === 0 && warnings.length === 0}
			<p class="muted">{t(locale, 'wizard.noFindings')}</p>
		{:else}
			<ul class="findings">
				{#each [...blocking, ...warnings] as finding, index (index)}
					<li class:blocking={finding.severity === 'blocking'}>
						<strong>
							{finding.severity === 'blocking'
								? t(locale, 'wizard.blocking')
								: t(locale, 'wizard.warning')}
						</strong>
						<span class="mono">{finding.code}</span>
						<span>{finding.detail}</span>
						{#if finding.row_ref !== null}
							<span class="mono muted">row {finding.row_ref}</span>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}

		<div class="row">
			<button class="btn btn-primary" disabled={!ready} onclick={runImport}>
				{t(locale, 'wizard.import')}
			</button>
		</div>
	{/if}
</section>

<style>
	.card {
		padding: 1.2rem 1.4rem;
	}

	.head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.8rem;
	}

	.row {
		display: flex;
		align-items: center;
		gap: 0.8rem;
		flex-wrap: wrap;
		margin: 0.6rem 0;
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
		gap: 0.8rem;
	}

	fieldset {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.7rem 0.9rem 0.9rem;
	}

	legend {
		font-weight: 600;
		padding: 0 0.3rem;
	}

	label,
	.label {
		display: block;
		margin-top: 0.5rem;
		font-size: 0.85rem;
	}

	select,
	input[type='number'],
	input[type='text'] {
		width: 100%;
	}

	.keys {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem 0.9rem;
		margin-top: 0.3rem;
	}

	.check {
		display: flex;
		align-items: center;
		gap: 0.35rem;
		margin-top: 0;
	}

	.small {
		font-size: 0.8rem;
	}

	.sample {
		max-height: 16rem;
		overflow: auto;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
	}

	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.85rem;
	}

	th,
	td {
		text-align: left;
		padding: 0.25rem 0.5rem;
		border-bottom: 1px solid var(--border);
	}

	.findings {
		margin: 0.4rem 0 0.8rem;
		padding-left: 1.2rem;
		font-size: 0.85rem;
	}

	.findings li {
		display: flex;
		gap: 0.5rem;
		flex-wrap: wrap;
	}

	.findings li.blocking {
		color: var(--danger, #b3261e);
	}
</style>
