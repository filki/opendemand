/**
 * Pure logic for the weekly planning workflow: the import -> forecast step
 * decision and the comparison labels. The Svelte components render these
 * results; keeping them here makes them testable without a DOM (ADR-0022).
 */
import type { ForecastChangePoint } from '#lib/bindings.ts';
import { formatQuantity, humanizeToken } from './format.ts';
import { sortRunsNewestFirst, type RunSummaryLike } from './jobs.ts';

export type WeeklyStep = { kind: 'reuse'; runId: string } | { kind: 'forecast' };

/**
 * After an import completes, reuse the revision's newest completed run; start
 * a forecast only when none exists. Duplicates never create redundant runs.
 */
export function resolveWeeklyStep(runs: RunSummaryLike[], revisionId: number): WeeklyStep {
	const completed = sortRunsNewestFirst(
		runs.filter((run) => run.revision_id === revisionId && run.status === 'completed')
	);
	const newest = completed[0];
	return newest ? { kind: 'reuse', runId: newest.id } : { kind: 'forecast' };
}

export function presenceLabel(presence: string): string | null {
	switch (presence) {
		case 'both':
			return null;
		case 'current_only':
			return 'Only in new revision';
		case 'previous_only':
			return 'Only in previous revision';
		default:
			return humanizeToken(presence);
	}
}

/**
 * How one forecast comparison row reads: horizon edges are visible but never
 * changes; in-horizon gaps are availability changes without a numeric delta.
 */
export function forecastPointNote(
	point: Pick<ForecastChangePoint, 'comparability' | 'is_change'>
): string {
	if (point.comparability === 'common') return '';
	if (point.is_change) {
		return point.comparability === 'previous_only'
			? 'Not forecast in the current run'
			: 'Not forecast in the previous run';
	}
	return 'Horizon edge, not compared';
}

export function formatDelta(value: number | null | undefined): string {
	if (typeof value !== 'number' || !Number.isFinite(value)) return '—';
	const formatted = formatQuantity(Math.abs(value));
	if (value > 0) return `+${formatted}`;
	if (value < 0) return `-${formatted}`;
	return formatted;
}
