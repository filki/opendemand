/**
 * Workspace view helpers: a workspace that has not been configured yet is a
 * normal empty state, not an error, so the first run can offer create/open.
 */
import type { Locale } from './i18n.ts';

export function isNoWorkspaceError(error: unknown): boolean {
	if (typeof error !== 'object' || error === null) return false;
	return (error as { code?: unknown }).code === 'no_workspace';
}

export async function persistLocaleForWorkspace(
	workspacePath: string,
	locale: Locale,
	currentWorkspacePath: () => string | null | undefined,
	saveLocale: (locale: Locale) => Promise<unknown>,
	refreshWorkspace: () => Promise<void>
): Promise<boolean> {
	if (currentWorkspacePath() !== workspacePath) return false;
	await saveLocale(locale);
	if (currentWorkspacePath() !== workspacePath) return false;
	await refreshWorkspace();
	return true;
}
