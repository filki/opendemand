import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	buildChartModel,
	describeChart,
	formatWeekLabel,
	mergeTableRows,
	niceTicks,
	planValueSourceLabel,
	weekIndex,
	weekIndexToIso
} from '../../src/lib/frontend/chart.ts';
import type { ForecastPointInput, HistoryPointInput, PlanningPointInput } from '../../src/lib/frontend/chart.ts';

const WEEK0 = '2026-01-05'; // a Monday

function addWeeks(iso: string, count: number): string {
	const index = weekIndex(iso);
	assert.ok(index !== null, `expected a valid ISO date: ${iso}`);
	return weekIndexToIso(index + count);
}

function weekIndexOf(offset: number): number {
	const index = weekIndex(addWeeks(WEEK0, offset));
	assert.ok(index !== null, `expected a valid week offset: ${offset}`);
	return index;
}

function planningPoint(weekOffset: number, fields: Partial<PlanningPointInput> = {}): PlanningPointInput {
	return {
		week_start: addWeeks(WEEK0, weekOffset),
		horizon: weekOffset + 1,
		baseline: null,
		override_value: null,
		effective: null,
		value_source: 'missing',
		override_revision: 0,
		...fields
	};
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

test('effective plan and baseline share one y-domain scaled with history', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' },
		{ week_start: addWeeks(WEEK0, 1), quantity: 10, status: 'known' }
	];
	const planning: PlanningPointInput[] = [
		planningPoint(2, { baseline: 12, effective: 12, value_source: 'model' }),
		planningPoint(3, {
			baseline: 13,
			override_value: 24,
			effective: 24,
			value_source: 'override',
			override_revision: 1
		})
	];
	const model = buildChartModel(history, [], undefined, planning);
	assert.equal(model.planningWeeks, 2);
	assert.ok(model.domain.y0 < 10, 'the domain must include the history level');
	assert.ok(model.domain.y1 > 24, 'the domain must include the override value');
	assert.equal(model.planningBaselinePaths.length, 1);
	assert.equal(model.effectivePaths.length, 1);
	assert.equal(model.overridePoints.length, 1);
	const x = (offset: number) =>
		model.plot.x + ((weekIndexOf(offset) - model.domain.x0) / (model.domain.x1 - model.domain.x0)) * model.plot.width;
	const y = (value: number) =>
		model.plot.y + model.plot.height - ((value - model.domain.y0) / (model.domain.y1 - model.domain.y0)) * model.plot.height;
	assert.ok(Math.abs(model.overridePoints[0].x - x(3)) < 1e-9);
	assert.ok(Math.abs(model.overridePoints[0].y - y(24)) < 1e-9);
});

test('overridden plan cells are highlighted and stay distinct from modeled cells', () => {
	const planning: PlanningPointInput[] = [
		planningPoint(0, { baseline: 10, effective: 10, value_source: 'model' }),
		planningPoint(1, {
			baseline: 10,
			override_value: 30,
			effective: 30,
			value_source: 'override',
			override_revision: 2
		}),
		planningPoint(2, { baseline: 10, effective: 10, value_source: 'model' })
	];
	const model = buildChartModel([], [], undefined, planning);
	assert.equal(model.effectivePaths.length, 1);
	assert.equal(model.isolatedEffectivePoints.length, 0);
	assert.equal(model.overridePoints.length, 1);
	const x = (offset: number) =>
		model.plot.x + ((weekIndexOf(offset) - model.domain.x0) / (model.domain.x1 - model.domain.x0)) * model.plot.width;
	const y = (value: number) =>
		model.plot.y + model.plot.height - ((value - model.domain.y0) / (model.domain.y1 - model.domain.y0)) * model.plot.height;
	assert.ok(Math.abs(model.overridePoints[0].x - x(1)) < 1e-9);
	assert.ok(Math.abs(model.overridePoints[0].y - y(30)) < 1e-9);
	assert.ok(model.overridePoints[0].y < y(10), 'the override marker sits above the modeled level');
});

test('a manual-only plan value plots alone without a fabricated baseline', () => {
	const planning: PlanningPointInput[] = [
		planningPoint(0),
		planningPoint(1),
		planningPoint(2, { override_value: 5, effective: 5, value_source: 'override', override_revision: 1 }),
		planningPoint(3),
		planningPoint(4)
	];
	const model = buildChartModel([], [], undefined, planning);
	assert.equal(model.isEmpty, false);
	assert.equal(model.planningBaselinePaths.length, 0);
	assert.equal(model.isolatedPlanningBaselinePoints.length, 0);
	assert.equal(model.effectivePaths.length, 0);
	assert.equal(model.isolatedEffectivePoints.length, 1);
	assert.equal(model.overridePoints.length, 1);
	assert.equal(model.zeroY, null, 'a lone value of 5 must not be zero-filled');
	assert.equal(model.domain.x0, weekIndexOf(0));
	assert.equal(model.domain.x1, weekIndexOf(4));
	assert.ok(model.domain.y0 < 5 && model.domain.y1 > 5);
});

test('missing and cleared plan weeks break the baseline and effective lines instead of bridging', () => {
	const planning: PlanningPointInput[] = [
		planningPoint(0, { override_value: 10, effective: 10, value_source: 'override', override_revision: 1 }),
		planningPoint(1, { override_value: 11, effective: 11, value_source: 'override', override_revision: 1 }),
		planningPoint(2),
		planningPoint(3, {
			baseline: 12,
			override_value: null,
			effective: 12,
			value_source: 'model',
			override_revision: 3
		}),
		planningPoint(4, { baseline: 13, effective: 13, value_source: 'model' })
	];
	const model = buildChartModel([], [], undefined, planning);
	assert.equal(model.effectivePaths.length, 2);
	assert.deepEqual(
		model.effectivePaths.map((path) => path.split('L').length),
		[2, 2],
		'each effective segment spans exactly its two weeks'
	);
	assert.equal(model.planningBaselinePaths.length, 1);
	assert.equal(model.planningBaselinePaths[0].split('L').length, 2);
	assert.equal(model.isolatedPlanningBaselinePoints.length, 0);
	assert.equal(model.overridePoints.length, 2);
	assert.equal(model.gapCount, 0, 'plan gaps are not history gaps');
	const missingX =
		model.plot.x + ((weekIndexOf(2) - model.domain.x0) / (model.domain.x1 - model.domain.x0)) * model.plot.width;
	for (const path of [...model.effectivePaths, ...model.planningBaselinePaths]) {
		const xs = [...path.matchAll(/[ML] (-?[\d.]+)/g)].map((match) => Number(match[1]));
		assert.ok(
			xs.every((value) => Math.abs(value - missingX) > 0.5),
			`no vertex on the cleared week: ${path}`
		);
	}
});

test('the x-axis keeps the full planning horizon even when trailing weeks are missing', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' },
		{ week_start: addWeeks(WEEK0, 1), quantity: 11, status: 'known' }
	];
	const planning: PlanningPointInput[] = [
		planningPoint(2, { baseline: 12, effective: 12, value_source: 'model' }),
		planningPoint(3),
		planningPoint(4),
		planningPoint(5)
	];
	const model = buildChartModel(history, [], undefined, planning);
	assert.equal(model.weekCount, 6);
	assert.equal(model.domain.x1, weekIndexOf(5));
	assert.equal(model.weekTicks[model.weekTicks.length - 1].index, weekIndexOf(5));
	assert.equal(model.planningWeeks, 4);
});

test('adding planning does not change the persisted forecast baseline geometry', () => {
	const history: HistoryPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' },
		{ week_start: addWeeks(WEEK0, 1), quantity: 11, status: 'known' }
	];
	const forecast: ForecastPointInput[] = [
		{ week_start: addWeeks(WEEK0, 2), value: 12 },
		{ week_start: addWeeks(WEEK0, 3), value: 13 },
		{ week_start: addWeeks(WEEK0, 4), value: 12.5 }
	];
	const planning: PlanningPointInput[] = [
		planningPoint(2, { baseline: 12, effective: 12, value_source: 'model' }),
		planningPoint(3, { baseline: 13, effective: 13, value_source: 'model' }),
		planningPoint(4, { baseline: 12.5, effective: 12.5, value_source: 'model' })
	];
	const before = buildChartModel(history, forecast);
	const after = buildChartModel(history, forecast, undefined, planning);
	assert.equal(after.forecastPath, before.forecastPath);
	assert.equal(after.splitX, before.splitX);
	assert.equal(after.connector, before.connector);
	assert.deepEqual(after.domain, before.domain);
	assert.deepEqual(after.valueTicks, before.valueTicks);
	assert.equal(after.planningWeeks, 3);
	assert.equal(after.effectivePaths.length, 1);
	assert.equal(after.planningBaselinePaths.length, 1);
});

test('table rows add plan rows after forecast rows of the same week', () => {
	const history: HistoryPointInput[] = [{ week_start: addWeeks(WEEK0, 0), quantity: 10, status: 'known' }];
	const forecast: ForecastPointInput[] = [{ week_start: addWeeks(WEEK0, 1), value: 12 }];
	const planning: PlanningPointInput[] = [
		planningPoint(1, {
			baseline: 12,
			override_value: 20,
			effective: 20,
			value_source: 'override',
			override_revision: 1
		}),
		planningPoint(2)
	];
	const rows = mergeTableRows(history, forecast, planning);
	assert.deepEqual(
		rows.map((row) => `${row.week_start}:${row.kind}`),
		[
			`${addWeeks(WEEK0, 0)}:history`,
			`${addWeeks(WEEK0, 1)}:forecast`,
			`${addWeeks(WEEK0, 1)}:plan`,
			`${addWeeks(WEEK0, 2)}:plan`
		]
	);
	const overrideRow = rows.find((row) => row.kind === 'plan' && row.week_start === addWeeks(WEEK0, 1));
	assert.equal(overrideRow?.value, 20);
	assert.equal(overrideRow?.status, 'override');
});

test('plan value source labels are human readable', () => {
	assert.equal(planValueSourceLabel('override'), 'Manual override');
	assert.equal(planValueSourceLabel('model'), 'Model');
	assert.equal(planValueSourceLabel('missing'), 'Missing');
});

test('chart summary describes the rendered baseline and effective plan', () => {
	const forecast: ForecastPointInput[] = [
		{ week_start: addWeeks(WEEK0, 0), value: 12 },
		{ week_start: addWeeks(WEEK0, 1), value: 12 }
	];
	const planning: PlanningPointInput[] = [
		planningPoint(0, { baseline: 12, effective: 12, value_source: 'model' }),
		planningPoint(1, {
			baseline: 12,
			override_value: 20,
			effective: 20,
			value_source: 'override',
			override_revision: 1
		})
	];
	const model = buildChartModel([], forecast, undefined, planning);
	const summary = describeChart('SKU-1', [], forecast, model, planning);
	assert.match(summary, /Baseline: 2 weekly points, drawn dashed\./);
	assert.match(summary, /1 manually overridden week/);
	assert.match(summary, /Effective values are drawn solid; manual overrides are marked\./);
	assert.doesNotMatch(summary, /Forecast:.*drawn dashed after the split/);
});
