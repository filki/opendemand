import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	buildChartModel,
	formatWeekLabel,
	mergeTableRows,
	niceTicks,
	weekIndex,
	weekIndexToIso
} from '../../src/lib/frontend/chart.ts';
import type { ForecastPointInput, HistoryPointInput } from '../../src/lib/frontend/chart.ts';

const WEEK0 = '2026-01-05'; // a Monday

function addWeeks(iso: string, count: number): string {
	const index = weekIndex(iso);
	assert.ok(index !== null, `expected a valid ISO date: ${iso}`);
	return weekIndexToIso(index + count);
}

test('weekIndex maps consecutive Mondays to consecutive integers', () => {
	assert.equal(weekIndex('2026-01-05'), weekIndex('2026-01-12')! - 1);
	assert.equal(weekIndexToIso(weekIndex('2026-01-05')!), '2026-01-05');
	assert.equal(weekIndex('not-a-date'), null);
	assert.equal(weekIndex('2026-13-01'), null);
});

test('week labels are deterministic', () => {
	assert.equal(formatWeekLabel('2026-01-05'), 'Jan 5');
	assert.equal(formatWeekLabel('2026-01-05', true), 'Jan 5, 2026');
});

test('niceTicks produces round steps covering the span', () => {
	assert.deepEqual(niceTicks(0, 100, 5), [0, 20, 40, 60, 80, 100]);
});

test('niceTicks handles negative spans', () => {
	assert.deepEqual(niceTicks(-15, -5, 4), [-15, -10, -5]);
});

test('niceTicks stays bounded on very large spans', () => {
	const ticks = niceTicks(0, 1_000_000, 5);
	assert.ok(ticks.length <= 8);
	assert.equal(ticks[0], 0);
	assert.equal(ticks[ticks.length - 1], 1_000_000);
});

test('constant series still gets a nonzero y-domain and readable ticks', () => {
	const history: HistoryPointInput[] = [0, 1, 2, 3].map((offset) => ({
		week_start: addWeeks(WEEK0, offset),
		quantity: 5,
		status: 'known'
	}));
	const model = buildChartModel(history, []);
	assert.equal(model.isEmpty, false);
	assert.ok(model.domain.y1 > model.domain.y0);
	assert.ok(model.valueTicks.length >= 2);
	assert.equal(model.historyPaths.length, 1);
});

test('history gaps break the line and are never filled with zeros', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' },
		{ week_start: addWeeks(WEEK0, 1), quantity: 12, status: 'known' },
		{ week_start: addWeeks(WEEK0, 2), quantity: null, status: 'missing' },
		{ week_start: addWeeks(WEEK0, 3), quantity: 8, status: 'known' },
		{ week_start: addWeeks(WEEK0, 4), quantity: 9, status: 'known' }
	];
	const model = buildChartModel(history, []);
	assert.equal(model.historyPaths.length, 2);
	assert.equal(model.gapCount, 1);
	assert.ok(!model.historyPaths.join(' ').includes('NaN'));
	// The padded domain is 8..12, so zero is outside it and no zero line is drawn.
	assert.equal(model.zeroY, null);
});

test('negative values keep zero inside the plot and draw the zero line', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: -5, status: 'return' },
		{ week_start: addWeeks(WEEK0, 1), quantity: 5, status: 'known' }
	];
	const model = buildChartModel(history, []);
	assert.ok(model.zeroY !== null);
	assert.ok(model.zeroY! >= model.plot.y && model.zeroY! <= model.plot.y + model.plot.height);
});

test('an isolated numeric week becomes a point marker', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: null, status: 'missing' },
		{ week_start: addWeeks(WEEK0, 1), quantity: 4, status: 'known' },
		{ week_start: addWeeks(WEEK0, 2), quantity: null, status: 'missing' }
	];
	const model = buildChartModel(history, []);
	assert.equal(model.historyPaths.length, 0);
	assert.equal(model.isolatedHistoryPoints.length, 1);
});

test('forecast becomes a dashed path after the split zone, connected when adjacent', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' },
		{ week_start: addWeeks(WEEK0, 1), quantity: 11, status: 'known' }
	];
	const forecast: ForecastPointInput[] = [
		{ week_start: addWeeks(WEEK0, 2), value: 12 },
		{ week_start: addWeeks(WEEK0, 3), value: 13 },
		{ week_start: addWeeks(WEEK0, 4), value: 12.5 }
	];
	const model = buildChartModel(history, forecast);
	assert.ok(model.forecastPath !== null);
	assert.ok(model.splitX !== null);
	assert.ok(model.forecastZone !== null);
	assert.ok(model.forecastZone.x <= model.splitX + 0.001);
	assert.ok(model.connector !== null);
});

test('no connector is drawn across a gap before the horizon', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' },
		{ week_start: addWeeks(WEEK0, 1), quantity: null, status: 'missing' }
	];
	const forecast: ForecastPointInput[] = [
		{ week_start: addWeeks(WEEK0, 3), value: 12 },
		{ week_start: addWeeks(WEEK0, 4), value: 13 }
	];
	const model = buildChartModel(history, forecast);
	assert.equal(model.connector, null);
	assert.ok(model.splitX !== null);
});

test('an empty series is flagged as empty instead of drawing a fake baseline', () => {
	const model = buildChartModel([], []);
	assert.equal(model.isEmpty, true);
	assert.equal(model.historyPaths.length, 0);
	assert.equal(model.forecastPath, null);
});

test('nullable JSON forecast values are never plotted as zeros', () => {
	const model = buildChartModel([], [{ week_start: '2026-01-05', value: null }]);
	assert.equal(model.isEmpty, true);
	assert.equal(model.forecastPath, null);
	assert.equal(model.isolatedForecastPoints.length, 0);
});

test('mergeTableRows keeps weeks chronological across history and forecast', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 1), quantity: 11, status: 'known' },
		{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' }
	];
	const forecast: ForecastPointInput[] = [{ week_start: addWeeks(WEEK0, 2), value: 12 }];
	const rows = mergeTableRows(history, forecast);
	assert.deepEqual(
		rows.map((row) => `${row.week_start}:${row.kind}`),
		[
			`${addWeeks(WEEK0, 0)}:history`,
			`${addWeeks(WEEK0, 1)}:history`,
			`${addWeeks(WEEK0, 2)}:forecast`
		]
	);
});
