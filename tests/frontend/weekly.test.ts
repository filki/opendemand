import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { RunSummaryLike } from '../../src/lib/frontend/jobs.ts';
import {
	forecastPointNote,
	formatDelta,
	presenceLabel,
	resolveWeeklyStep
} from '../../src/lib/frontend/weekly.ts';

function run(overrides: Partial<RunSummaryLike> & { id: string }): RunSummaryLike {
	return {
		revision_id: 1,
		status: 'completed',
		created_at: '2026-02-02T10:00:00Z',
		horizon_start_week: '2026-02-02',
		engine_version: '0.0.1',
		...overrides
	};
}

test('weekly step forecasts a fresh revision and reuses the newest completed run on a duplicate', () => {
	assert.deepEqual(resolveWeeklyStep([], 2), { kind: 'forecast' });
	assert.deepEqual(
		resolveWeeklyStep(
			[
				run({ id: 'older', revision_id: 2, created_at: '2026-01-01T10:00:00Z' }),
				run({ id: 'newer', revision_id: 2, created_at: '2026-03-01T10:00:00Z' })
			],
			2
		),
		{ kind: 'reuse', runId: 'newer' }
	);
});

test('weekly step ignores runs of other revisions and unfinished runs', () => {
	assert.deepEqual(
		resolveWeeklyStep(
			[
				run({ id: 'other-revision', revision_id: 1 }),
				run({ id: 'failed', revision_id: 2, status: 'failed' }),
				run({ id: 'running', revision_id: 2, status: 'running' }),
				run({ id: 'cancelled', revision_id: 2, status: 'cancelled' })
			],
			2
		),
		{ kind: 'forecast' }
	);
});

test('presence labels name the side a series only exists on', () => {
	assert.equal(presenceLabel('both'), null);
	assert.equal(presenceLabel('current_only'), 'Only in new revision');
	assert.equal(presenceLabel('previous_only'), 'Only in previous revision');
});

test('forecast notes distinguish availability changes from horizon edges', () => {
	assert.equal(forecastPointNote({ comparability: 'common', is_change: true }), '');
	assert.equal(forecastPointNote({ comparability: 'common', is_change: false }), '');
	assert.equal(
		forecastPointNote({ comparability: 'previous_only', is_change: true }),
		'Not forecast in the current run'
	);
	assert.equal(
		forecastPointNote({ comparability: 'current_only', is_change: true }),
		'Not forecast in the previous run'
	);
	assert.equal(
		forecastPointNote({ comparability: 'previous_only', is_change: false }),
		'Horizon edge, not compared'
	);
	assert.equal(
		forecastPointNote({ comparability: 'current_only', is_change: false }),
		'Horizon edge, not compared'
	);
});

test('formatDelta signs values and keeps a dash for missing deltas', () => {
	assert.equal(formatDelta(null), '—');
	assert.equal(formatDelta(Number.NaN), '—');
	assert.equal(formatDelta(2.5), '+2.5');
	assert.equal(formatDelta(-3), '-3');
	assert.equal(formatDelta(0), '0');
});
