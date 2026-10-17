<script lang="ts">
	import type { WorkspaceInfo } from '#lib/bindings.ts';
	import { LOCALES, LOCALE_NAMES, t, type Locale } from '#lib/frontend/i18n.ts';

	let {
		workspace,
		appVersion,
		jobBusy,
		locale,
		localeBusy,
		onSwitch,
		onImport,
		onGuidedImport,
		onForecast,
		onLocaleChange
	}: {
		workspace: WorkspaceInfo;
		appVersion: string | null;
		jobBusy: boolean;
		locale: Locale;
		localeBusy: boolean;
		onSwitch: () => void;
		onImport: () => void;
		onGuidedImport: () => void;
		onForecast: (imputationMethod: string | null, modelSet: string) => void;
		onLocaleChange: (value: Locale) => void;
	} = $props();

	let forecastDialog: HTMLDialogElement;
	let fillMissingWeeks = $state(false);
	let imputationMethod = $state('last');
	let modelSet = $state('default');

	function handleLocaleSelect(event: Event) {
		onLocaleChange((event.currentTarget as HTMLSelectElement).value as Locale);
	}

	function openForecastDialog() {
		fillMissingWeeks = false;
		modelSet = 'default';
		forecastDialog.showModal();
	}

	function confirmForecast() {
		onForecast(fillMissingWeeks ? imputationMethod : null, modelSet);
		forecastDialog.close();
	}
</script>

<section class="panel section" aria-labelledby="workspace-heading">
	<header class="heading">
		<div>
			<h2 id="workspace-heading">Workspace</h2>
			<p class="subtitle muted">Explore imported sales history and compare forecasts.</p>
		</div>
		<button class="btn btn-secondary" onclick={onSwitch} disabled={jobBusy}>
			Switch workspace
		</button>
	</header>

	<dl class="meta">
		<div class="location">
			<dt>Workspace path</dt>
			<dd class="mono path" title={workspace.path} translate="no">{workspace.path}</dd>
		</div>
		<div>
			<dt>Workspace schema</dt>
			<dd>{workspace.schema_version}</dd>
		</div>
		<div>
			<dt>Latest import</dt>
			<dd>{workspace.latest_revision_id === null ? 'Not imported yet' : `#${workspace.latest_revision_id}`}</dd>
		</div>
		<div>
			<dt>OpenDemand version</dt>
			<dd>{appVersion === null ? 'Loading…' : appVersion === 'Unavailable' ? appVersion : `v${appVersion}`}</dd>
		</div>
	</dl>

	{#if workspace.recovery_issues.length > 0}
		<div class="banner banner-warn recovery" role="alert">
			<p class="recovery-title">Recovery findings ({workspace.recovery_issues.length})</p>
			<ul>
				{#each workspace.recovery_issues as issue}
					<li>{issue}</li>
				{/each}
			</ul>
		</div>
	{/if}

	<div class="actions">
		<button class="btn btn-secondary" onclick={onImport} disabled={jobBusy}>Import CSV</button>
		<button class="btn btn-secondary" onclick={onGuidedImport} disabled={jobBusy}>Guided import</button>
		<button
			class="btn btn-primary"
			onclick={openForecastDialog}
			disabled={jobBusy || workspace.latest_revision_id === null}
		>
			Generate forecast
		</button>
	</div>
	<p class="hint muted">
		Use Guided import for files that need column mapping. Import history and generate forecasts separately, then compare runs to explore changes.
	</p>

	<details class="preferences">
		<summary>Workspace preferences</summary>
		<div class="preferences-content">
			<label class="fill-field" for="workspace-locale">
				<span>{t(locale, 'workspace.language')}</span>
				<select
					id="workspace-locale"
					name="workspace-locale"
					autocomplete="off"
					value={locale}
					disabled={jobBusy || localeBusy}
					onchange={handleLocaleSelect}
				>
					{#each LOCALES as option (option)}
						<option value={option}>{LOCALE_NAMES[option]}</option>
					{/each}
				</select>
			</label>
			<p class="hint muted">{t(locale, 'workspace.languageHint')}</p>
		</div>
	</details>
</section>

<dialog
	bind:this={forecastDialog}
	class="forecast-dialog"
	aria-labelledby="forecast-dialog-heading"
	aria-describedby="forecast-dialog-description"
>
	<div class="dialog-content">
		<h2 id="forecast-dialog-heading">Check missing weeks</h2>
		<p id="forecast-dialog-description" class="muted">
			Missing weeks are not treated as zero. Choose whether to estimate them for this forecast. Your imported history will not change.
		</p>

		<fieldset>
			<legend>Missing weeks</legend>
			<label class="choice">
				<input
					type="radio"
					name="missing-weeks"
					value="leave"
					checked={!fillMissingWeeks}
					onchange={() => (fillMissingWeeks = false)}
				/>
				<span>Leave them as they are</span>
			</label>
			<label class="choice">
				<input
					type="radio"
					name="missing-weeks"
					value="fill"
					checked={fillMissingWeeks}
					onchange={() => (fillMissingWeeks = true)}
				/>
				<span>Estimate them for this forecast</span>
			</label>
		</fieldset>

		{#if fillMissingWeeks}
			<label class="fill-field method-field" for="imputation-method">
				<span>Estimation method</span>
				<select
					id="imputation-method"
					name="imputation-method"
					autocomplete="off"
					value={imputationMethod}
					onchange={(event) => (imputationMethod = (event.currentTarget as HTMLSelectElement).value)}
				>
					<option value="last">Use the last reported week</option>
					<option value="last_seasonal_52">Use the same week last year</option>
					<option value="mean_4">Use the average of the last four reported weeks</option>
				</select>
			</label>
		{/if}

		<label class="fill-field method-field" for="model-set">
			<span>Model set</span>
			<select
				id="model-set"
				name="model-set"
				autocomplete="off"
				value={modelSet}
				onchange={(event) => (modelSet = (event.currentTarget as HTMLSelectElement).value)}
			>
				<option value="default">Naive, SeasonalNaive, AutoETS</option>
				<option value="sparse">Add Croston-SBA and TSB for intermittent demand</option>
			</select>
			<span class="muted small">
				Sparse candidates need a complete, non-negative history; other series keep the default
				models.
			</span>
		</label>

		<div class="dialog-actions">
			<button class="btn btn-secondary" onclick={() => forecastDialog.close()}>Cancel</button>
			<button class="btn btn-primary" onclick={confirmForecast} disabled={jobBusy}>
				Generate forecast
			</button>
		</div>
	</div>
</dialog>

<style>
	.section {
		padding: 1.15rem 1.3rem;
	}

	.heading {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 1rem;
	}

	h2 {
		margin-bottom: 0.25rem;
	}

	.subtitle {
		margin: 0;
	}

	.meta {
		display: grid;
		grid-template-columns: minmax(0, 2fr) repeat(3, minmax(8rem, 1fr));
		gap: 0.85rem 1.25rem;
		margin: 1rem 0;
		padding: 0.9rem 0;
		border-block: 1px solid var(--border);
	}

	.meta div {
		min-width: 0;
	}

	.meta dt {
		color: var(--ink-muted);
		font-size: 0.78rem;
		font-weight: 600;
	}

	.meta dd {
		margin: 0.25rem 0 0;
		font-size: 0.92rem;
		font-weight: 600;
	}

	.path {
		font-size: 0.82rem;
		font-weight: 500;
		overflow-wrap: anywhere;
	}

	.recovery {
		margin-bottom: 0.8rem;
	}

	.recovery-title {
		margin: 0 0 0.3rem;
		font-weight: 600;
	}

	.recovery ul {
		margin: 0;
		padding-left: 1.1rem;
	}

	.actions {
		display: flex;
		justify-content: flex-end;
		flex-wrap: wrap;
		gap: 0.6rem;
	}

	.hint {
		margin: 0.45rem 0 0;
		font-size: 0.82rem;
	}

	.preferences {
		margin-top: 0.8rem;
	}

	.preferences summary {
		width: fit-content;
		color: var(--ink-soft);
		font-size: 0.88rem;
		font-weight: 600;
		cursor: pointer;
	}

	.preferences-content {
		max-width: 38rem;
		padding-top: 0.75rem;
	}

	.fill-field {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		color: var(--ink-soft);
		font-size: 0.85rem;
		font-weight: 600;
	}

	.fill-field select {
		min-height: 40px;
		padding: 0.4rem 0.6rem;
		border: 1px solid var(--border-strong);
		border-radius: var(--radius-sm);
		background: var(--surface);
		color: var(--ink);
		font: inherit;
		font-weight: 400;
	}

	.forecast-dialog {
		width: min(35rem, calc(100vw - 2rem));
		max-height: calc(100vh - 2rem);
		overflow: auto;
		overscroll-behavior: contain;
		padding: 0;
		border: 1px solid var(--border-strong);
		border-radius: 12px;
		background: var(--surface);
		color: var(--ink);
		box-shadow: 0 20px 60px rgba(21, 34, 56, 0.24);
	}

	.forecast-dialog::backdrop {
		background: rgba(21, 34, 56, 0.48);
	}

	.dialog-content {
		padding: 1.4rem;
	}

	.dialog-content > p {
		max-width: 58ch;
	}

	fieldset {
		margin: 1rem 0;
		padding: 0;
		border: 0;
	}

	legend {
		margin-bottom: 0.35rem;
		font-size: 0.85rem;
		font-weight: 650;
	}

	.choice {
		display: flex;
		align-items: flex-start;
		gap: 0.65rem;
		margin-top: 0.5rem;
		padding: 0.7rem 0.8rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}

	.choice input {
		margin: 0.3rem 0 0;
		accent-color: var(--cobalt);
	}

	.method-field {
		margin: 0 0 1rem;
	}

	.dialog-actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.6rem;
		margin-top: 1.25rem;
	}

	@media (max-width: 820px) {
		.meta {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	@media (max-width: 560px) {
		.section {
			padding: 1rem;
		}

		.heading {
			flex-direction: column;
		}

		.meta {
			grid-template-columns: 1fr;
		}

		.actions > button {
			flex: 1;
		}

		.dialog-content {
			padding: 1.1rem;
		}

		.dialog-actions {
			flex-direction: column-reverse;
		}
	}
</style>
