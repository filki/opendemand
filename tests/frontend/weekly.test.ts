import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	changedListEmptyText,
	changeKindLabel,
	forecastPointNote,
	formatDelta,
	observationStatusText,
	outcomeText,
	planSourceText,
	presenceLabel,
	WEEKLY_CHANGE_FILTERS,
	weeklyChangeFilterLabel
} from '../../src/lib/frontend/weekly.ts';

test('presence labels name the side a series only exists on', () => {
	assert.equal(presenceLabel('en', 'both'), null);
	assert.equal(presenceLabel('en', 'current_only'), 'Only in new revision');
	assert.equal(presenceLabel('en', 'previous_only'), 'Only in previous revision');
	assert.equal(presenceLabel('pl', 'current_only'), 'Tylko w nowej rewizji');
	assert.equal(presenceLabel('es', 'previous_only'), 'Solo en la revisión anterior');
});

test('forecast notes distinguish availability changes from horizon edges', () => {
	assert.equal(forecastPointNote('en', { comparability: 'common', is_change: true }), '');
	assert.equal(forecastPointNote('en', { comparability: 'common', is_change: false }), '');
	assert.equal(
		forecastPointNote('en', { comparability: 'previous_only', is_change: true }),
		'Not forecast in the current run'
	);
	assert.equal(
		forecastPointNote('en', { comparability: 'current_only', is_change: true }),
		'Not forecast in the previous run'
	);
	assert.equal(
		forecastPointNote('en', { comparability: 'previous_only', is_change: false }),
		'Horizon edge, not compared'
	);
	assert.equal(
		forecastPointNote('en', { comparability: 'current_only', is_change: false }),
		'Horizon edge, not compared'
	);
	assert.equal(
		forecastPointNote('pl', { comparability: 'previous_only', is_change: true }),
		'Brak prognozy w bieżącym przebiegu'
	);
});

test('change-type filter options keep the Rust order and translate labels', () => {
	assert.deepEqual(WEEKLY_CHANGE_FILTERS, ['all', 'demand', 'forecast', 'presence']);
	assert.equal(weeklyChangeFilterLabel('en', 'all'), 'All changes');
	assert.equal(weeklyChangeFilterLabel('en', 'demand'), 'Demand corrections');
	assert.equal(weeklyChangeFilterLabel('en', 'forecast'), 'Forecast changes');
	assert.equal(weeklyChangeFilterLabel('en', 'presence'), 'Arrivals and removals');
	assert.equal(weeklyChangeFilterLabel('pl', 'demand'), 'Korekty popytu');
	assert.equal(weeklyChangeFilterLabel('es', 'presence'), 'Series añadidas y eliminadas');
});

test('empty comparison states separate a filtered-out list from nothing changed', () => {
	assert.equal(changedListEmptyText('en', '', 'all'), 'No series changed between these two runs.');
	assert.equal(changedListEmptyText('en', 'widget', 'all'), 'No changed series match this search.');
	assert.equal(changedListEmptyText('en', '', 'demand'), 'No changes match this filter.');
	assert.equal(changedListEmptyText('en', 'widget', 'forecast'), 'No changes match this filter.');
	assert.equal(changedListEmptyText('pl', '', 'presence'), 'Żadne zmiany nie pasują do tego filtra.');
});

test('comparison labels translate outcomes, statuses, change kinds and sources', () => {
	assert.equal(outcomeText('en', 'fallback_naive'), 'Naive fallback');
	assert.equal(outcomeText('en', null), 'Not forecast');
	assert.equal(outcomeText('pl', 'insufficient_history'), 'Zbyt krótka historia');
	assert.equal(outcomeText('es', 'invalid_data'), 'Datos no válidos');
	assert.equal(outcomeText('en', 'custom_outcome'), 'Custom Outcome');
	assert.equal(observationStatusText('en', 'stockout'), 'Stockout');
	assert.equal(observationStatusText('pl', 'zero'), 'Zero');
	assert.equal(observationStatusText('es', null), '—');
	assert.equal(changeKindLabel('en', 'added'), 'Added');
	assert.equal(changeKindLabel('pl', 'removed'), 'Usunięto');
	assert.equal(changeKindLabel('es', 'changed'), 'Modificado');
	assert.equal(planSourceText('en', 'override'), 'Manual override');
	assert.equal(planSourceText('pl', 'model'), 'Model');
	assert.equal(planSourceText('es', 'missing'), 'Faltante');
	assert.equal(planSourceText('en', 'future_source'), 'future_source');
});

test('formatDelta signs values and keeps a dash for missing deltas', () => {
	assert.equal(formatDelta(null), '—');
	assert.equal(formatDelta(Number.NaN), '—');
	assert.equal(formatDelta(2.5), '+2.5');
	assert.equal(formatDelta(-3), '-3');
	assert.equal(formatDelta(0), '0');
});
