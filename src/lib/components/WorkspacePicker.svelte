<script lang="ts">
	import type { WorkspaceRef } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import { formatTimestamp } from '#lib/frontend/format.ts';
	import {
		canManageWorkspace,
		confirmWorkspaceName,
		isWorkspaceNameValid,
		isWorkspaceOpenable,
		isWorkspaceRemovable
	} from '#lib/frontend/home.ts';

	let {
		entries,
		loading,
		busy,
		onCreate,
		onOpenDirectory,
		onOpenPath,
		onForget,
		onRename,
		onDelete,
		onBack
	}: {
		entries: WorkspaceRef[];
		loading: boolean;
		busy: boolean;
		onCreate: () => void;
		onOpenDirectory: () => void;
		onOpenPath: (path: string) => void;
		onForget: (path: string) => void;
		onRename: (path: string, name: string) => Promise<boolean>;
		onDelete: (path: string, name: string) => Promise<boolean>;
		onBack: () => void;
	} = $props();

	let renamePath = $state<string | null>(null);
	let renameValue = $state('');
	let deleteEntry = $state<WorkspaceRef | null>(null);
	let deleteStage = $state<'sure' | 'type'>('sure');
	let deleteValue = $state('');
	let managementPending = $state(false);
	let controlsBusy = $derived(busy || managementPending);

	function startRename(entry: WorkspaceRef) {
		renamePath = entry.path;
		renameValue = entry.name;
	}

	function cancelRename() {
		renamePath = null;
		renameValue = '';
	}

	async function submitRename(entry: WorkspaceRef) {
		if (controlsBusy || !isWorkspaceNameValid(renameValue)) return;
		managementPending = true;
		try {
			if (await onRename(entry.path, renameValue.trim())) cancelRename();
		} finally {
			managementPending = false;
		}
	}

	function startDelete(entry: WorkspaceRef) {
		deleteEntry = entry;
		deleteStage = 'sure';
		deleteValue = '';
	}

	function cancelDelete() {
		deleteEntry = null;
		deleteValue = '';
	}

	async function submitDelete() {
		if (controlsBusy || deleteEntry === null) return;
		if (!confirmWorkspaceName(deleteValue, deleteEntry.name)) return;
		managementPending = true;
		try {
			if (await onDelete(deleteEntry.path, deleteValue)) cancelDelete();
		} finally {
			managementPending = false;
		}
	}
</script>

<section class="panel card picker" aria-labelledby="picker-heading">
	<div class="head">
		<button class="btn btn-secondary" onclick={onBack} disabled={controlsBusy}>← Home</button>
		<div class="titles">
			<h2 id="picker-heading">Forecasting workspaces</h2>
			<p class="muted">Open a recent workspace, or create one in the default location.</p>
		</div>
	</div>

	<div class="row actions">
		<button class="btn btn-primary" onclick={onCreate} disabled={controlsBusy}>Create workspace</button>
		<button class="btn btn-secondary" onclick={onOpenDirectory} disabled={controlsBusy}>Open folder…</button>
	</div>

	{#if loading}
		<p class="muted" role="status">Loading workspaces…</p>
	{:else if entries.length === 0}
		<p class="muted">
			No workspaces yet. Create one in the default location, or open an existing workspace folder.
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

					{#if renamePath === entry.path}
						<div class="row rename-form">
							<label>
								<span class="muted">New name</span>
								<input
									type="text"
									value={renameValue}
									disabled={controlsBusy}
									oninput={(event) => (renameValue = event.currentTarget.value)}
									onkeydown={(event) => {
										if (event.key === 'Enter') submitRename(entry);
										if (event.key === 'Escape') cancelRename();
									}}
								/>
							</label>
							<button
								class="btn btn-primary"
								disabled={controlsBusy || !isWorkspaceNameValid(renameValue)}
								onclick={() => submitRename(entry)}
							>
								Rename
							</button>
							<button class="btn btn-secondary" disabled={controlsBusy} onclick={cancelRename}>Cancel</button>
						</div>
					{:else}
						<div class="row entry-actions">
							{#if !isWorkspaceOpenable(entry)}
								<StatusChip label="Not found" tone="warn" />
							{:else if entry.is_default}
								<StatusChip label="Default location" tone="neutral" />
							{/if}
							<button
								class="btn btn-primary"
								disabled={controlsBusy || !isWorkspaceOpenable(entry)}
								onclick={() => onOpenPath(entry.path)}
							>
								Open
							</button>
							{#if canManageWorkspace(entry)}
								<button class="btn btn-secondary" disabled={controlsBusy} onclick={() => startRename(entry)}>
									Rename…
								</button>
								<button class="btn btn-secondary" disabled={controlsBusy} onclick={() => startDelete(entry)}>
									Delete…
								</button>
							{/if}
							{#if isWorkspaceRemovable(entry)}
								<button class="btn btn-secondary" disabled={controlsBusy} onclick={() => onForget(entry.path)}>
									Remove
								</button>
							{/if}
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}

	{#if deleteEntry}
		<div class="delete-dialog" role="alertdialog" aria-labelledby="delete-heading">
			<h3 id="delete-heading">Delete “{deleteEntry.name}”?</h3>
			<p>
				This permanently deletes the whole workspace folder, including the database, retained
				sources, exports, and backups. This cannot be undone.
			</p>
			{#if deleteStage === 'sure'}
				<div class="row">
					<button class="btn btn-danger" disabled={controlsBusy} onclick={() => (deleteStage = 'type')}>
						Yes, delete…
					</button>
					<button class="btn btn-secondary" disabled={controlsBusy} onclick={cancelDelete}>Cancel</button>
				</div>
			{:else}
				<label class="confirm-field">
					<span>Type <strong>{deleteEntry.name}</strong> to confirm</span>
					<input
						type="text"
						value={deleteValue}
						disabled={controlsBusy}
						oninput={(event) => (deleteValue = event.currentTarget.value)}
						onkeydown={(event) => {
							if (event.key === 'Enter') submitDelete();
							if (event.key === 'Escape') cancelDelete();
						}}
					/>
				</label>
				<div class="row">
					<button
						class="btn btn-danger"
						disabled={controlsBusy || !confirmWorkspaceName(deleteValue, deleteEntry.name)}
						onclick={submitDelete}
					>
						Delete permanently
					</button>
					<button class="btn btn-secondary" disabled={controlsBusy} onclick={cancelDelete}>Cancel</button>
				</div>
			{/if}
		</div>
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

	.row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.45rem;
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

	.rename-form {
		flex-shrink: 0;
		align-items: flex-end;
		gap: 0.45rem;
	}

	.rename-form label {
		display: grid;
		gap: 0.2rem;
		font-size: 0.78rem;
	}

	.rename-form input {
		min-width: 12rem;
	}

	.delete-dialog {
		display: grid;
		gap: 0.6rem;
		padding: 0.9rem 1rem;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--surface-subtle);
	}

	.delete-dialog h3 {
		margin: 0;
	}

	.delete-dialog p {
		margin: 0;
		color: var(--ink-muted);
	}

	.confirm-field {
		display: grid;
		gap: 0.25rem;
	}

	.confirm-field input {
		max-width: 22rem;
	}
</style>
