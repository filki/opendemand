<script lang="ts">
	import type { WorkspaceInfo } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';

	let {
		workspace,
		revisionId,
		jobBusy,
		onCreate,
		onOpen,
		onImport,
		onForecast
	}: {
		workspace: WorkspaceInfo | null;
		revisionId: number | null;
		jobBusy: boolean;
		onCreate: () => void;
		onOpen: () => void;
		onImport: () => void;
		onForecast: () => void;
	} = $props();
</script>

<section class="panel section" aria-labelledby="workspace-heading">
	<h2 id="workspace-heading">Workspace</h2>

	{#if workspace}
		<dl class="meta">
			<div>
				<dt>Path</dt>
				<dd class="mono path" title={workspace.path}>{workspace.path}</dd>
			</div>
			<div>
				<dt>Schema</dt>
				<dd><StatusChip label={workspace.schema_version} tone="neutral" /></dd>
			</div>
			<div>
				<dt>Latest revision</dt>
				<dd>{revisionId !== null ? revisionId : 'None yet'}</dd>
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
			<button class="btn btn-primary btn-block" onclick={onImport} disabled={jobBusy}>
				Import CSV…
			</button>
			<button
				class="btn btn-secondary btn-block"
				onclick={onForecast}
				disabled={jobBusy || revisionId === null}
			>
				Generate Naive forecast
			</button>
			<button class="btn btn-secondary btn-block" onclick={onOpen} disabled={jobBusy}>
				Open workspace…
			</button>
			<button class="btn btn-secondary btn-block" onclick={onCreate} disabled={jobBusy}>
				Create workspace
			</button>
			<p class="hint muted">Create uses the application data directory. Switching workspaces is refused while a job runs.</p>
		</div>
	{:else}
		<p class="muted">No workspace is open. Create or open one from the main panel.</p>
	{/if}
</section>

<style>
	.section {
		padding: 1rem 1.1rem;
	}

	h2 {
		margin-bottom: 0.6rem;
	}

	.meta {
		margin: 0 0 0.8rem;
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
		font-size: 0.88rem;
	}

	.meta div {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 0.6rem;
	}

	.meta dt {
		color: var(--ink-muted);
		font-weight: 600;
		flex-shrink: 0;
	}

	.meta dd {
		margin: 0;
		text-align: right;
		min-width: 0;
	}

	.path {
		word-break: break-all;
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
		flex-direction: column;
		gap: 0.45rem;
	}

	.hint {
		margin: 0.2rem 0 0;
		font-size: 0.78rem;
	}
</style>
