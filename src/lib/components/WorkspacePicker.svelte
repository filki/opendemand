<script lang="ts">
	import type { WorkspaceRef } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import { formatTimestamp } from '#lib/frontend/format.ts';
	import { isWorkspaceOpenable } from '#lib/frontend/home.ts';

	let {
		entries,
		loading,
		busy,
		onCreate,
		onOpenDirectory,
		onOpenPath,
		onForget,
		onBack
	}: {
		entries: WorkspaceRef[];
		loading: boolean;
		busy: boolean;
		onCreate: () => void;
		onOpenDirectory: () => void;
		onOpenPath: (path: string) => void;
		onForget: (path: string) => void;
		onBack: () => void;
	} = $props();
</script>

<section class="panel card picker" aria-labelledby="picker-heading">
	<div class="head">
		<button class="btn btn-secondary" onclick={onBack}>← Home</button>
		<div class="titles">
			<h2 id="picker-heading">Forecasting workspaces</h2>
			<p class="muted">Open a recent workspace, or start a new one in an empty folder.</p>
		</div>
	</div>

	<div class="row actions">
		<button class="btn btn-primary" onclick={onCreate} disabled={busy}>Create workspace…</button>
		<button class="btn btn-secondary" onclick={onOpenDirectory} disabled={busy}>Open folder…</button>
	</div>

	{#if loading}
		<p class="muted" role="status">Loading workspaces…</p>
	{:else if entries.length === 0}
		<p class="muted">
			No workspaces yet. Create one in an empty folder, or open an existing workspace folder.
		</p>
	{:else}
		<ul class="list">
			{#each entries as entry (entry.path)}
				<li class="entry">
					<div class="meta">
						<span class="name">{entry.name}</span>
						<span class="mono path" title={entry.path}>{entry.path}</span>
						<span class="muted when">
							{entry.last_opened_at
								? `Last opened ${formatTimestamp(entry.last_opened_at)}`
								: 'Default location'}
						</span>
					</div>
					<div class="row entry-actions">
						{#if !isWorkspaceOpenable(entry)}
							<StatusChip label="Not found" tone="warn" />
						{/if}
						<button
							class="btn btn-primary"
							disabled={busy || !isWorkspaceOpenable(entry)}
							onclick={() => onOpenPath(entry.path)}
						>
							Open
						</button>
						<button class="btn btn-secondary" disabled={busy} onclick={() => onForget(entry.path)}>
							Remove
						</button>
					</div>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	.picker {
		padding: 1.2rem 1.4rem;
		display: grid;
		gap: 0.9rem;
	}

	.head {
		display: flex;
		align-items: flex-start;
		gap: 0.9rem;
	}

	.titles h2 {
		margin: 0;
	}

	.titles p {
		margin: 0.25rem 0 0;
	}

	.actions {
		gap: 0.6rem;
	}

	.list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0.55rem;
	}

	.entry {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
		padding: 0.7rem 0.9rem;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--surface-subtle);
	}

	.meta {
		display: grid;
		gap: 0.15rem;
		min-width: 0;
	}

	.name {
		font-weight: 600;
	}

	.path {
		font-size: 0.82rem;
		color: var(--ink-muted);
		word-break: break-all;
	}

	.when {
		font-size: 0.78rem;
	}

	.entry-actions {
		flex-shrink: 0;
		gap: 0.45rem;
	}
</style>
