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
