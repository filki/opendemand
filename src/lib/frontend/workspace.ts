/**
 * Workspace view helpers: a workspace that has not been configured yet is a
 * normal empty state, not an error, so the first run can offer create/open.
 */

export function isNoWorkspaceError(error: unknown): boolean {
	if (typeof error !== 'object' || error === null) return false;
	return (error as { code?: unknown }).code === 'no_workspace';
}
