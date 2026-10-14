<script lang="ts">
	let { onPickModule }: { onPickModule: (id: string) => void } = $props();
</script>

<section class="panel card home" aria-labelledby="home-heading">
	<div class="intro">
		<h2 id="home-heading">Choose a module</h2>
		<p>
			OpenDemand is a local-first workspace: your data stays on this machine. Pick a module to open
			one of your workspaces.
		</p>
	</div>
	<div class="modules">
		<button class="module" type="button" onclick={() => onPickModule('forecasting')}>
			<span class="module-title">Forecasting</span>
			<span class="module-copy">
				Import weekly sales, forecast demand per SKU and location, plan adjustments, and compare
				runs.
			</span>
			<span class="module-action">Open workspaces →</span>
		</button>
	</div>
</section>

<style>
	.home {
		padding: 1.4rem;
		display: grid;
		gap: 1rem;
	}

	.intro p {
		margin: 0.35rem 0 0;
		color: var(--ink-muted);
		max-width: 44rem;
	}

	.modules {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 0.8rem;
	}

	.module {
		display: grid;
		gap: 0.35rem;
		text-align: left;
		padding: 1rem 1.1rem;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--surface);
		color: inherit;
		cursor: pointer;
	}

	.module:hover,
	.module:focus-visible {
		border-color: var(--cobalt);
	}

	.module-title {
		font-weight: 700;
		font-size: 1.05rem;
	}

	.module-copy {
		color: var(--ink-muted);
		font-size: 0.88rem;
	}

	.module-action {
		color: var(--cobalt);
		font-size: 0.85rem;
		font-weight: 600;
	}
</style>
