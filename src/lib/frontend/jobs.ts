/**
 * Job and run helpers: terminal-state detection for polling, progress display,
 * and run selection for restoring the UI after a reload.
 */

import { humanizeToken } from './format.ts';

export interface JobInfoLike {
	id: string;
	kind: string;
	status: string;
	phase: string;
	progress: number;
}

export interface RunSummaryLike {
	id: string;
	revision_id: number;
	status: string;
	created_at: string;
	horizon_start_week: string;
	engine_version: string;
}

const NON_TERMINAL_STATUSES = new Set(['queued', 'running']);

export function isTerminalJobStatus(status: string | null | undefined): boolean {
	if (status === null || status === undefined || status === '') return false;
	return !NON_TERMINAL_STATUSES.has(status.toLowerCase());
}

export function isActiveJobStatus(status: string | null | undefined): boolean {
	return !isTerminalJobStatus(status);
}

/** Rust progress is a fraction in the range 0..1. */
export function progressPercent(progress: number | null | undefined): number {
	if (progress === null || progress === undefined || !Number.isFinite(progress)) return 0;
	const percent = progress * 100;
	return Math.max(0, Math.min(100, Math.round(percent)));
}

export function jobKindLabel(kind: string): string {
	switch (kind) {
		case 'import':
			return 'CSV import';
		case 'forecast':
			return 'Naive forecast';
		default:
			return humanizeToken(kind);
	}
}

export function jobStatusLabel(status: string): string {
	switch (status.toLowerCase()) {
		case 'queued':
			return 'Queued';
		case 'running':
			return 'Running';
		case 'completed':
			return 'Completed';
		case 'failed':
			return 'Failed';
		case 'cancelled':
			return 'Cancelled';
		default:
			return humanizeToken(status);
	}
}

export function sortRunsNewestFirst<T extends RunSummaryLike>(runs: T[]): T[] {
	return [...runs].sort((a, b) => {
		const ta = Date.parse(a.created_at);
		const tb = Date.parse(b.created_at);
		if (Number.isNaN(ta) || Number.isNaN(tb)) return 0;
		return tb - ta;
	});
}

/** Explicit selection wins; automatic selection stays on the latest import. */
export function selectDisplayRun<T extends RunSummaryLike>(
	runs: T[],
	revisionId?: number | null,
	selectedRunId?: string | null
): T | null {
	const selected = runs.find((run) => run.id === selectedRunId);
	if (selected) return selected;
	const sorted = sortRunsNewestFirst(
		revisionId === undefined ? runs : runs.filter((run) => run.revision_id === revisionId)
	);
	return sorted.find((run) => run.status === 'completed') ?? null;
}
