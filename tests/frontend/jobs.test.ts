import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	isActiveJobStatus,
	isTerminalJobStatus,
	jobKindLabel,
	jobStatusLabel,
	progressPercent,
	selectDisplayRun,
	sortRunsNewestFirst
} from '../../src/lib/frontend/jobs.ts';
import type { RunSummaryLike } from '../../src/lib/frontend/jobs.ts';

test('job statuses: non-terminal while queued or running, terminal afterwards', () => {
	assert.equal(isTerminalJobStatus('queued'), false);
	assert.equal(isTerminalJobStatus('running'), false);
	assert.equal(isTerminalJobStatus('RUNNING'), false);
	assert.equal(isTerminalJobStatus('completed'), true);
	assert.equal(isTerminalJobStatus('failed'), true);
	assert.equal(isTerminalJobStatus('cancelled'), true);
	assert.equal(isTerminalJobStatus(null), false);
	assert.equal(isActiveJobStatus(null), true);
	assert.equal(isActiveJobStatus('completed'), false);
});

test('progressPercent uses fractional progress and clamps invalid values', () => {
	assert.equal(progressPercent(0.25), 25);
	assert.equal(progressPercent(1), 100);
	assert.equal(progressPercent(40), 100);
	assert.equal(progressPercent(-1), 0);
	assert.equal(progressPercent(Number.NaN), 0);
});

test('labels stay sentence case', () => {
	assert.equal(jobKindLabel('import'), 'CSV import');
	assert.equal(jobKindLabel('forecast'), 'Naive forecast');
	assert.equal(jobStatusLabel('queued'), 'Queued');
	assert.equal(jobStatusLabel('cancelled'), 'Cancelled');
});

test('run selection prefers the newest completed run', () => {
	const runs: RunSummaryLike[] = [
		{
			id: 'run-b',
			revision_id: 2,
			status: 'completed',
			created_at: '2026-02-02T10:00:00Z',
			horizon_start_week: '2026-02-02',
			engine_version: '0.1.0'
		},
		{
			id: 'run-a',
			revision_id: 1,
			status: 'completed',
			created_at: '2026-01-01T10:00:00Z',
			horizon_start_week: '2026-01-05',
			engine_version: '0.1.0'
		},
		{
			id: 'run-c',
			revision_id: 3,
			status: 'failed',
			created_at: '2026-03-01T10:00:00Z',
			horizon_start_week: '2026-03-02',
			engine_version: '0.1.0'
		}
	];
	assert.equal(sortRunsNewestFirst(runs)[0].id, 'run-c');
	assert.equal(selectDisplayRun(runs)?.id, 'run-b');
	assert.equal(selectDisplayRun([runs[2]]), null);
});
