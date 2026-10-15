/**
 * Home screen helpers: the module menu leads to the workspace list, which
 * renders recent workspaces and offers create/open actions.
 */
import type { WorkspaceRef } from '#lib/bindings.ts';

/** One-time guidance shown under a failed create, matched to the IPC code. */
export function createWorkspaceHint(code: string): string | null {
	switch (code) {
		case 'workspace_exists':
			return 'A workspace already lives in that folder. Open it from the list instead.';
		case 'workspace_not_empty':
			return 'Choose an empty folder for a new workspace.';
		case 'workspace_not_directory':
			return 'Choose a folder, not a file.';
		default:
			return null;
	}
}

/** A list entry is openable only while its workspace database exists. */
export function isWorkspaceOpenable(entry: WorkspaceRef): boolean {
	return entry.exists;
}

/**
 * The default application-data workspace is always re-listed by the backend,
 * so removing its entry never sticks; only recent entries are removable.
 */
export function isWorkspaceRemovable(entry: WorkspaceRef): boolean {
	return !entry.is_default;
}

/**
 * Rename and delete act on the workspace directory, so they are offered only
 * for a workspace whose database exists and that is not the pinned default.
 */
export function canManageWorkspace(entry: WorkspaceRef): boolean {
	return entry.exists && !entry.is_default;
}

/** The typed name must match the workspace name exactly, like GitHub. */
export function confirmWorkspaceName(typed: string, name: string): boolean {
	return typed === name;
}

/** A new workspace name must be a single, non-empty folder name. */
export function isWorkspaceNameValid(value: string): boolean {
	const name = value.trim();
	return (
		name.length > 0 &&
		name !== '.' &&
		name !== '..' &&
		name.length <= 128 &&
		!/[\/\\\0]/.test(name)
	);
}

/** One-time guidance shown under a failed rename or delete, matched to the IPC code. */
export function workspaceActionHint(code: string): string | null {
	switch (code) {
		case 'invalid_workspace_name':
			return 'Use a single folder name without path separators.';
		case 'workspace_name_taken':
			return 'A folder with that name already exists next to this workspace.';
		case 'confirm_name_mismatch':
			return 'Type the workspace name exactly to confirm deletion.';
		case 'workspace_default':
			return 'The default workspace cannot be renamed or deleted.';
		case 'workspace_busy':
			return 'Close the open workspace and finish active jobs first.';
		default:
			return null;
	}
}
