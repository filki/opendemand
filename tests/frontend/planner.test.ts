import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	authoritativeInput,
	buildClearEdit,
	buildOverrideEdit,
	classifySaveFailure,
	createDraftState,
	dropDraft,
	isDraftDirty,
	isPlannerDirty,
	planningIdentity,
	rebaseDraft,
	rebaseDrafts,
	setDraft,
	syncDraftState
} from '../../src/lib/frontend/planner.ts';
import type { PlanningDetailLike, PlanningPointLike } from '../../src/lib/frontend/planner.ts';

function point(week: string, override: number | null, revision: number): PlanningPointLike {
	return { week_start: week, override_value: override, override_revision: revision };
}

function detail(overrides: Partial<PlanningDetailLike> = {}): PlanningDetailLike {
	return {
		run_id: 'run-1',
		product_id: 'product-1',
		location_id: 'location-1',
		planning: [point('2026-03-02', 24.5, 1), point('2026-03-09', null, 0)],
		...overrides
	};
}

test('authoritativeInput keeps the exact stored decimals and maps null to empty', () => {
	assert.equal(authoritativeInput(null), '');
	assert.equal(authoritativeInput(5), '5');
	assert.equal(authoritativeInput(24.5), '24.5');
	assert.equal(authoritativeInput(-2.5), '-2.5');
	assert.equal(authoritativeInput(0.1 + 0.2), '0.30000000000000004');
	assert.equal(authoritativeInput(1 / 3), String(1 / 3));
});

test('planningIdentity separates runs, products and locations', () => {
	const base = detail();
	assert.equal(planningIdentity(base), planningIdentity(detail()));
	assert.notEqual(planningIdentity(base), planningIdentity(detail({ run_id: 'run-2' })));
	assert.notEqual(planningIdentity(base), planningIdentity(detail({ product_id: 'product-2' })));
	assert.notEqual(planningIdentity(base), planningIdentity(detail({ location_id: 'location-2' })));
	assert.notEqual(planningIdentity(detail({ run_id: null })), planningIdentity(base));
});

test('draft state resets on an identity change and survives same-identity refreshes', () => {
	const original = createDraftState(detail());
	const drafted = setDraft(original, '2026-03-02', ' 30.25 ', 1);
	assert.equal(drafted.drafts['2026-03-02'].raw, ' 30.25 ');
	assert.equal(drafted.drafts['2026-03-02'].expectedRevision, 1);

	const refreshed = detail({
		planning: [point('2026-03-02', 30.25, 4), point('2026-03-09', null, 0)]
	});
	const kept = syncDraftState(drafted, refreshed);
	assert.equal(kept, drafted, 'same identity must keep the same state object, never rebase');
	assert.equal(kept.drafts['2026-03-02'].raw, ' 30.25 ');
	assert.equal(kept.drafts['2026-03-02'].expectedRevision, 1, 'background refresh must not rebase');

	const switched = syncDraftState(drafted, detail({ run_id: 'run-2' }));
	assert.notEqual(switched, drafted);
	assert.deepEqual(switched.drafts, {}, 'overrides cannot transfer to another run or series');
});

test('setDraft captures the expected revision once and keeps raw input verbatim', () => {
	let state = createDraftState(detail());
	state = setDraft(state, '2026-03-02', '24.5', 1);
	state = setDraft(state, '2026-03-02', '24.50', 1);
	assert.equal(state.drafts['2026-03-02'].raw, '24.50');
	assert.equal(state.drafts['2026-03-02'].expectedRevision, 1);

	state = setDraft(state, '2026-03-09', 'abc', 0);
	assert.equal(state.drafts['2026-03-09'].raw, 'abc', 'invalid strings are drafts too');

	const same = setDraft(state, '2026-03-02', '24.50', 1);
	assert.equal(same, state, 'a no-op edit keeps the same state object');
});

test('dropDraft removes only the requested week', () => {
	let state = createDraftState(detail());
	state = setDraft(state, '2026-03-02', '7', 1);
	state = setDraft(state, '2026-03-09', '8', 0);

	const dropped = dropDraft(state, '2026-03-02');
	assert.equal(dropped.drafts['2026-03-02'], undefined);
	assert.equal(dropped.drafts['2026-03-09'].raw, '8');
	assert.equal(dropDraft(dropped, '2026-04-06'), dropped, 'dropping a missing week is a no-op');
});

test('dirty tracking compares the raw input with the authoritative override text', () => {
	const clean = createDraftState(detail());
	assert.equal(isPlannerDirty(clean, detail()), false);

	let state = setDraft(clean, '2026-03-02', '24.5', 1);
	assert.equal(isDraftDirty(state, point('2026-03-02', 24.5, 1)), false, 'same text is not dirty');
	assert.equal(isPlannerDirty(state, detail()), false);

	state = setDraft(state, '2026-03-02', 'abc', 1);
	assert.equal(isDraftDirty(state, point('2026-03-02', 24.5, 1)), true, 'invalid strings are dirty');
	assert.equal(isPlannerDirty(state, detail()), true);

	const refreshed = detail({ planning: [point('2026-03-02', null, 0), point('2026-03-09', null, 0)] });
	assert.equal(isDraftDirty(state, refreshed.planning[0]), true, 'draft text differs from a cleared cell');

	assert.equal(
		isPlannerDirty(setDraft(clean, '2026-03-02', '9', 1), detail({ product_id: 'other' })),
		false,
		'drafts from another identity are not dirty for this series'
	);
});

test('buildOverrideEdit sends the exact raw input against the captured revision', () => {
	const edit = buildOverrideEdit(detail(), point('2026-03-02', 24.5, 1), ' 7.5 ', 1);
	assert.deepEqual(edit, {
		run_id: 'run-1',
		product_id: 'product-1',
		location_id: 'location-1',
		week_start: '2026-03-02',
		value: ' 7.5 ',
		expected_revision: 1
	});
});

test('buildClearEdit asks for an explicit NULL clear against the latest revision', () => {
	const edit = buildClearEdit(detail(), point('2026-03-02', 24.5, 3));
	assert.deepEqual(edit, {
		run_id: 'run-1',
		product_id: 'product-1',
		location_id: 'location-1',
		week_start: '2026-03-02',
		value: null,
		expected_revision: 3
	});
});

test('classifySaveFailure flags override conflicts and keeps messages', () => {
	const conflict = classifySaveFailure({
		code: 'override_conflict',
		message: 'This week changed since it was loaded.'
	});
	assert.equal(conflict.conflict, true);
	assert.equal(conflict.code, 'override_conflict');
	assert.equal(conflict.message, 'This week changed since it was loaded.');

	const invalid = classifySaveFailure({ code: 'invalid_value', message: 'Enter a finite numeric value.' });
	assert.equal(invalid.conflict, false);
	assert.equal(invalid.code, 'invalid_value');

	const thrown = classifySaveFailure(new Error('ipc exploded'));
	assert.equal(thrown.conflict, false);
	assert.equal(thrown.code, 'unknown_error');
	assert.equal(thrown.message, 'ipc exploded');

	assert.equal(classifySaveFailure('offline').message, 'offline');
	assert.equal(classifySaveFailure(undefined).code, 'unknown_error');
});

test('rebaseDrafts keeps raw input and updates expected revisions on explicit refresh', () => {
	let state = createDraftState(detail());
	state = setDraft(state, '2026-03-02', ' 30 ', 1);
	state = setDraft(state, '2026-03-09', 'bad input', 0);

	const fresh = detail({
		planning: [point('2026-03-02', 31.5, 2)]
	});
	const rebased = rebaseDrafts(state, fresh);
	assert.equal(rebased.drafts['2026-03-02'].raw, ' 30 ', 'raw input survives the refresh');
	assert.equal(rebased.drafts['2026-03-02'].expectedRevision, 2, 'explicit refresh rebases the revision');
	assert.equal(rebased.drafts['2026-03-09'].raw, 'bad input');
	assert.equal(
		rebased.drafts['2026-03-09'].expectedRevision,
		0,
		'a week missing from fresh data keeps its revision for review'
	);

	const switched = rebaseDrafts(state, detail({ run_id: 'run-9' }));
	assert.deepEqual(switched.drafts, {}, 'refreshing another identity never transfers drafts');
});

test('rebaseDraft updates one surviving draft after an explicit write', () => {
	let state = createDraftState(detail());
	state = setDraft(state, '2026-03-02', '30', 1);
	state = setDraft(state, '2026-03-09', '4', 0);

	const rebased = rebaseDraft(state, '2026-03-02', 2);
	assert.equal(rebased.drafts['2026-03-02'].expectedRevision, 2);
	assert.equal(rebased.drafts['2026-03-02'].raw, '30');
	assert.equal(rebased.drafts['2026-03-09'].expectedRevision, 0, 'other drafts stay untouched');

	assert.equal(rebaseDraft(rebased, '2026-03-02', 2), rebased, 'same revision is a no-op');
	assert.equal(rebaseDraft(rebased, '2026-04-06', 5), rebased, 'a week without a draft is a no-op');
});
