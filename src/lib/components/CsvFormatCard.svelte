<script lang="ts">
	let {
		onImport,
		onGuidedImport,
		guidedLabel
	}: { onImport?: () => void; onGuidedImport?: () => void; guidedLabel?: string } = $props();

	let showActions = $derived(onImport !== undefined || onGuidedImport !== undefined);
</script>

<section class="panel card" class:split={showActions} aria-labelledby="csv-format-heading">
	<div class="reference">
		<div class="intro">
			<h2 id="csv-format-heading">Import a canonical CSV</h2>
			<p>
				The first import creates revision 1. The file must be UTF-8, comma-separated, with exactly
				this header:
			</p>
			<pre><code>series_key,week_start,quantity</code></pre>
		</div>
		<div class="details">
			<ul>
				<li>
					<strong>series_key</strong> — nonempty, exact, case-sensitive identity for one product ×
					location series.
				</li>
				<li>
					<strong>week_start</strong> — ISO Monday date, for example <span class="mono">2026-10-05</span>.
				</li>
				<li>
					<strong>quantity</strong> — finite number; zero is explicit zero, negative values are returns.
				</li>
			</ul>
			<p class="muted">
				Identical duplicate cells collapse; conflicting cells are excluded with an import issue. Empty
				files and invalid rows fail validation before any revision is committed. The forecast cutoff is
				the maximum imported week, not today's date.
			</p>
		</div>
	</div>
	{#if showActions}
		<div class="actions">
			{#if onImport}
				<div class="row">
					<button class="btn btn-primary" onclick={onImport}>Import CSV…</button>
					{#if onGuidedImport}
						<button class="btn btn-secondary" onclick={onGuidedImport}>
							{guidedLabel ?? 'Guided import…'}
						</button>
					{/if}
				</div>
			{/if}
			{#if onGuidedImport}
				<p class="muted">
					The guided import converts a raw SAP, Subiekt, Comarch, or Excel export: pick the file, map
					the columns, check the preview, and import. Nothing is written until you confirm.
				</p>
			{/if}
		</div>
	{/if}
</section>

<style>
	.card {
		padding: 1.2rem 1.4rem;
		display: grid;
		gap: 1.2rem 2.5rem;
		align-items: start;
	}

	.reference {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		gap: 1.2rem 2.5rem;
		align-items: start;
	}

	.card.split {
		grid-template-columns: minmax(0, 1fr) minmax(16rem, 22rem);
	}

	.card.split .reference {
		grid-template-columns: minmax(0, 1fr);
	}

	.reference p,
	.reference li {
		max-width: 76ch;
	}

	.actions {
		display: flex;
		flex-direction: column;
		gap: 0.9rem;
		padding: 0.9rem 1rem;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--surface-subtle);
	}

	.row {
		display: flex;
		align-items: center;
		gap: 0.8rem;
		flex-wrap: wrap;
	}

	.actions p {
		margin: 0;
	}

	pre {
		margin: 0 0 0.9rem;
		padding: 0.6rem 0.8rem;
		overflow-x: auto;
		background: var(--surface-subtle);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		font-family: var(--font-mono);
		font-size: 0.85rem;
	}

	ul {
		margin: 0 0 0.9rem;
		padding-left: 1.2rem;
	}

	li + li {
		margin-top: 0.3rem;
	}

	@media (max-width: 1220px) {
		.card.split {
			grid-template-columns: 1fr;
		}
	}

	@media (max-width: 980px) {
		.reference {
			grid-template-columns: 1fr;
		}
	}
</style>
