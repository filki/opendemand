/**
 * Pure draft state for the weekly plan editor: exact raw input, the revision a
 * draft was authored against, dirty tracking, and explicit-refresh rebasing.
 *
 * Drafts are pure data so they can be unit-tested directly with the Node test
 * runner. Rust owns numeric and domain validation; nothing here parses or
 * validates override values. The backend expects exact raw input plus the
 * revision the planner actually edited.
 */

export interface PlanningPointLike {
	week_start: string;
	override_value: number | null;
	override_revision: number;
}

export interface PlanningDetailLike {
	run_id: string | null;
	product_id: string;
	location_id: string;
	planning: PlanningPointLike[];
}

export interface PlanningDraft {
	/** Exact text the planner typed; never trimmed, formatted or parsed. */
	raw: string;
	/** Revision the draft was authored against; only explicit actions change it. */
	expectedRevision: number;
}

export interface PlannerDraftState {
	identity: string;
	drafts: Record<string, PlanningDraft>;
}

export interface OverrideEditPayload {
	run_id: string;
	product_id: string;
	location_id: string;
	week_start: string;
	value: string | null;
	expected_revision: number;
}

export interface SaveFailure {
	code: string;
	message: string;
	conflict: boolean;
}

const UNKNOWN_FAILURE: SaveFailure = {
	code: 'unknown_error',
	message: 'The change could not be saved.',
	conflict: false
};

/** Identity that scopes drafts: overrides can never transfer between these. */
export function planningIdentity(detail: PlanningDetailLike): string {
	return `${detail.run_id ?? ''}\u0000${detail.product_id}\u0000${detail.location_id}`;
}

/** Exact editable text of the stored override; null means "no override". */
export function authoritativeInput(value: number | null): string {
	return value === null ? '' : String(value);
}

export function createDraftState(detail: PlanningDetailLike): PlannerDraftState {
	return { identity: planningIdentity(detail), drafts: {} };
}

/**
 * Keep drafts only while the run/product/location identity is unchanged.
 * A same-identity refresh (background query update) never rebases revisions.
 */
export function syncDraftState(
	state: PlannerDraftState,
	detail: PlanningDetailLike
): PlannerDraftState {
	return state.identity === planningIdentity(detail) ? state : createDraftState(detail);
}

export function setDraft(
	state: PlannerDraftState,
	week: string,
	raw: string,
	expectedRevision: number
): PlannerDraftState {
	const existing = state.drafts[week];
	if (existing && existing.raw === raw && existing.expectedRevision === expectedRevision) {
		return state;
	}
	return {
		identity: state.identity,
		drafts: { ...state.drafts, [week]: { raw, expectedRevision } }
	};
}

/** Discard a draft: Escape, or commit it after a successful save. */
export function dropDraft(state: PlannerDraftState, week: string): PlannerDraftState {
	if (!state.drafts[week]) return state;
	const drafts = { ...state.drafts };
	delete drafts[week];
	return { identity: state.identity, drafts };
}

export function isDraftDirty(state: PlannerDraftState, point: PlanningPointLike): boolean {
	const draft = state.drafts[point.week_start];
	return draft !== undefined && draft.raw !== authoritativeInput(point.override_value);
}

export function isPlannerDirty(state: PlannerDraftState, detail: PlanningDetailLike): boolean {
	if (state.identity !== planningIdentity(detail)) return false;
	return detail.planning.some((point) => isDraftDirty(state, point));
}

/**
 * Explicit "refresh and review": keep every raw input, re-anchor each draft to
 * the authoritative revision. Only called for a user-requested refresh.
 */
export function rebaseDrafts(
	state: PlannerDraftState,
	fresh: PlanningDetailLike
): PlannerDraftState {
	const identity = planningIdentity(fresh);
	if (state.identity !== identity) return { identity, drafts: {} };
	const revisions = new Map(fresh.planning.map((point) => [point.week_start, point.override_revision]));
	const drafts: Record<string, PlanningDraft> = {};
	for (const [week, draft] of Object.entries(state.drafts)) {
		const revision = revisions.get(week);
		drafts[week] = revision === undefined ? draft : { raw: draft.raw, expectedRevision: revision };
	}
	return { identity, drafts };
}

/** Re-anchor one draft after a successful write to that same cell. */
export function rebaseDraft(
	state: PlannerDraftState,
	week: string,
	revision: number
): PlannerDraftState {
	const draft = state.drafts[week];
	if (!draft || draft.expectedRevision === revision) return state;
	return { identity: state.identity, drafts: { ...state.drafts, [week]: { raw: draft.raw, expectedRevision: revision } } };
}

export function buildOverrideEdit(
	detail: PlanningDetailLike,
	point: PlanningPointLike,
	raw: string,
	expectedRevision: number
): OverrideEditPayload {
	return {
		run_id: detail.run_id ?? '',
		product_id: detail.product_id,
		location_id: detail.location_id,
		week_start: point.week_start,
		value: raw,
		expected_revision: expectedRevision
	};
}

/** A NULL clear: explicit revert of the override, never a fabricated 0. */
export function buildClearEdit(
	detail: PlanningDetailLike,
	point: PlanningPointLike
): OverrideEditPayload {
	return {
		run_id: detail.run_id ?? '',
		product_id: detail.product_id,
		location_id: detail.location_id,
		week_start: point.week_start,
		value: null,
		expected_revision: point.override_revision
	};
}

/** Rust errors arrive as { code, message } (AppError); be lenient about the rest. */
export function classifySaveFailure(error: unknown): SaveFailure {
	if (error !== null && typeof error === 'object') {
		const record = error as { code?: unknown; message?: unknown };
		const code = typeof record.code === 'string' && record.code !== '' ? record.code : '';
		const message =
			typeof record.message === 'string' && record.message !== ''
				? record.message
				: code !== ''
					? code
					: UNKNOWN_FAILURE.message;
		return {
			code: code === '' ? UNKNOWN_FAILURE.code : code,
			message,
			conflict: code === 'override_conflict'
		};
	}
	if (typeof error === 'string' && error !== '') {
		return { code: UNKNOWN_FAILURE.code, message: error, conflict: false };
	}
	return UNKNOWN_FAILURE;
}
