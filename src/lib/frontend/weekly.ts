/**
 * Pure logic for comparing forecast runs and labeling weekly changes. The
 * Svelte components render these results; keeping them here makes them
 * testable without a DOM (ADR-0022). Labels use shared dictionaries so
 * en/pl/es follow the workspace locale.
 */
import type { ForecastChangePoint } from '#lib/bindings.ts';
import { formatQuantity, humanizeToken } from './format.ts';
import { t, type Locale, type MessageKey } from './i18n.ts';

/**
 * The typed change-type filter of the weekly comparison list. The variants
 * match `WeeklyChangeFilter` in Rust; `all` preserves the unfiltered result.
 */
export type WeeklyChangeFilter = 'all' | 'demand' | 'forecast' | 'presence';

export const WEEKLY_CHANGE_FILTERS: WeeklyChangeFilter[] = ['all', 'demand', 'forecast', 'presence'];

const FILTER_KEYS: Record<WeeklyChangeFilter, MessageKey> = {
	all: 'weekly.filter.all',
	demand: 'weekly.filter.demand',
	forecast: 'weekly.filter.forecast',
	presence: 'weekly.filter.presence'
};

export function weeklyChangeFilterLabel(locale: Locale, filter: WeeklyChangeFilter): string {
	return t(locale, FILTER_KEYS[filter]);
}

/**
 * The empty-list copy: a filtered-out list is a different state from a
 * comparison where nothing changed at all.
 */
export function changedListEmptyText(
	locale: Locale,
	search: string,
	filter: WeeklyChangeFilter
): string {
	if (filter !== 'all') return t(locale, 'weekly.noFilterMatch');
	return search.trim() !== '' ? t(locale, 'weekly.noSearchMatch') : t(locale, 'weekly.noChanges');
}

export function presenceLabel(locale: Locale, presence: string): string | null {
	switch (presence) {
		case 'both':
			return null;
		case 'current_only':
			return t(locale, 'weekly.presenceCurrentOnly');
		case 'previous_only':
			return t(locale, 'weekly.presencePreviousOnly');
		default:
			return humanizeToken(presence);
	}
}

/**
 * How one forecast comparison row reads: horizon edges are visible but never
 * changes; in-horizon gaps are availability changes without a numeric delta.
 */
export function forecastPointNote(
	locale: Locale,
	point: Pick<ForecastChangePoint, 'comparability' | 'is_change'>
): string {
	if (point.comparability === 'common') return '';
	if (point.is_change) {
		return point.comparability === 'previous_only'
			? t(locale, 'weekly.noteNotInCurrent')
			: t(locale, 'weekly.noteNotInPrevious');
	}
	return t(locale, 'weekly.noteHorizonEdge');
}

export function changeKindLabel(locale: Locale, kind: string): string {
	switch (kind) {
		case 'added':
			return t(locale, 'weekly.changeAdded');
		case 'removed':
			return t(locale, 'weekly.changeRemoved');
		case 'changed':
			return t(locale, 'weekly.changeChanged');
		default:
			return humanizeToken(kind);
	}
}

/** Localized outcome text for the comparison surfaces; unknown tokens humanize. */
export function outcomeText(locale: Locale, outcome: string | null | undefined): string {
	if (!outcome) return t(locale, 'outcome.none');
	switch (outcome) {
		case 'ok':
			return t(locale, 'outcome.ok');
		case 'fallback_naive':
			return t(locale, 'outcome.fallbackNaive');
		case 'insufficient_history':
			return t(locale, 'outcome.insufficientHistory');
		case 'model_failure':
			return t(locale, 'outcome.modelFailure');
		case 'invalid_data':
			return t(locale, 'outcome.invalidData');
		default:
			return humanizeToken(outcome);
	}
}

/** Localized observation-status text for the comparison surfaces. */
export function observationStatusText(locale: Locale, status: string | null | undefined): string {
	if (!status) return '—';
	switch (status) {
		case 'known':
			return t(locale, 'status.known');
		case 'zero':
			return t(locale, 'status.zero');
		case 'missing':
			return t(locale, 'status.missing');
		case 'return':
			return t(locale, 'status.return');
		case 'correction':
			return t(locale, 'status.correction');
		case 'stockout':
			return t(locale, 'status.stockout');
		case 'partial':
			return t(locale, 'status.partial');
		default:
			return humanizeToken(status);
	}
}

/** Localized planning value source; unknown tokens stay verbatim. */
export function planSourceText(locale: Locale, source: string): string {
	switch (source) {
		case 'override':
			return t(locale, 'source.override');
		case 'model':
			return t(locale, 'source.model');
		case 'missing':
			return t(locale, 'source.missing');
		default:
			return source;
	}
}

export function formatDelta(value: number | null | undefined): string {
	if (typeof value !== 'number' || !Number.isFinite(value)) return '—';
	const formatted = formatQuantity(Math.abs(value));
	if (value > 0) return `+${formatted}`;
	if (value < 0) return `-${formatted}`;
	return formatted;
}
