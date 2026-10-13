<script lang="ts">
	// Shown when the page runs in an ordinary browser. There is no fake backend:
	// the workspace database lives behind the Tauri desktop shell, so the honest
	// answer is to ask for the desktop application.
</script>

<main class="fallback">
	<div class="panel card">
		<p class="eyebrow">OpenDemand</p>
		<h1>Open in desktop application</h1>
		<p>
			OpenDemand is a local-first desktop application. Your workspace database, imports, and forecasts
			stay on this machine, reachable only through the local Rust core.
		</p>
		<p class="muted">
			Launch the installed OpenDemand desktop app to create or open a workspace, import a canonical CSV,
			and generate a Naive forecast.
		</p>
	</div>
</main>

<style>
	.fallback {
		display: flex;
		align-items: center;
		justify-content: center;
		min-height: 100vh;
		padding: 1.5rem;
	}

	.card {
		max-width: 30rem;
		padding: 1.6rem 1.8rem;
	}

	.eyebrow {
		margin: 0 0 0.4rem;
		color: var(--cobalt);
		font-size: 0.8rem;
		font-weight: 700;
		letter-spacing: 0.08em;
		text-transform: uppercase;
	}

	h1 {
		font-size: 1.4rem;
	}

	p:last-child {
		margin-bottom: 0;
	}
</style>
