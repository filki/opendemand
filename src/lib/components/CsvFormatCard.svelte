<script lang="ts">
	let {
		onImport,
		onGuidedImport,
		guidedLabel
	}: { onImport?: () => void; onGuidedImport?: () => void; guidedLabel?: string } = $props();
</script>

<section class="panel card" aria-labelledby="csv-format-heading">
	<h2 id="csv-format-heading">Import a canonical CSV</h2>
	<p>
		The first import creates revision 1. The file must be UTF-8, comma-separated, with exactly this
		header:
	</p>
	<pre><code>series_key,week_start,quantity</code></pre>
	<ul>
		<li>
			<strong>series_key</strong> — nonempty, exact, case-sensitive identity for one product × location
			series.
		</li>
		<li>
			<strong>week_start</strong> — ISO Monday date, for example <span class="mono">2026-10-05</span>.
		</li>
		<li>
			<strong>quantity</strong> — finite number; zero is explicit zero, negative values are returns.
		</li>
	</ul>
	<p class="muted">
		Identical duplicate cells collapse; conflicting cells are excluded with an import issue. Empty files
		and invalid rows fail validation before any revision is committed. The forecast cutoff is the maximum
		imported week, not today's date.
	</p>
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
</section>

<style>
	.card {
		padding: 1.2rem 1.4rem;
	}

	.row {
		display: flex;
		align-items: center;
		gap: 0.8rem;
		flex-wrap: wrap;
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
</style>
