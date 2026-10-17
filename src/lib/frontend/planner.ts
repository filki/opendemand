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
	override_reason?: string | null;
	override_note?: string | null;
	override_review_date?: string | null;
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

/** The required reason and optional note/review date of one override write. */
export interface OverrideDecision {
	reasonCode: string;
	note: string;
	reviewDate: string;
}

export interface OverrideEditPayload {
	run_id: string;
	product_id: string;
	location_id: string;
	week_start: string;
	value: string | null;
	expected_revision: number;
	reason_code: string;
	note: string | null;
	review_date: string | null;
}

export interface SeriesNotePayload {
	run_id: string;
	product_id: string;
	location_id: string;
	note: string;
}

export interface ReviewActionPayload {
	run_id: string;
	product_id: string;
	location_id: string;
	action: 'reviewed' | 'reopened';
}

export interface SaveFailure {
	code: string;
	message: string;
	conflict: boolean;
}

/**
 * The planner states that must block a series or run selection change: an
 * unsaved draft, an in-flight save, or an in-flight export. Both the series
 * list and the run-overview exceptions go through this one guard.
 */
export interface SelectionGuardState {
	dirty: boolean;
	saving: boolean;
	exporting: boolean;
}

export function canChangeSelection(state: SelectionGuardState): boolean {
	return !state.dirty && !state.saving && !state.exporting;
}

export interface RunExceptionSelectionState extends SelectionGuardState {
	selectedRunId: string | null;
	selectedProductId: string | null;
	selectedLocationId: string | null;
	comparisonOpen: boolean;
}

export interface RunExceptionTarget {
	runId: string;
	productId: string;
	locationId: string;
}

export function selectRunException(
	state: RunExceptionSelectionState,
	target: RunExceptionTarget
): (RunExceptionTarget & { comparisonOpen: false }) | null {
	if (!canChangeSelection(state)) return null;
	if (
		state.selectedRunId === target.runId &&
		state.selectedProductId === target.productId &&
		state.selectedLocationId === target.locationId &&
		!state.comparisonOpen
	) {
		return null;
	}
	return { ...target, comparisonOpen: false };
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

/** Empty note and review-date inputs become explicit NULLs; the reason stays required. */
function decidedFields(decision: OverrideDecision): {
	reason_code: string;
	note: string | null;
	review_date: string | null;
} {
	const note = decision.note.trim();
	const reviewDate = decision.reviewDate.trim();
	return {
		reason_code: decision.reasonCode,
		note: note === '' ? null : note,
		review_date: reviewDate === '' ? null : reviewDate
	};
}

export function buildOverrideEdit(
	detail: PlanningDetailLike,
	point: PlanningPointLike,
	raw: string,
	expectedRevision: number,
	decision: OverrideDecision
): OverrideEditPayload {
	return {
		run_id: detail.run_id ?? '',
		product_id: detail.product_id,
		location_id: detail.location_id,
		week_start: point.week_start,
		value: raw,
		expected_revision: expectedRevision,
		...decidedFields(decision)
	};
}

/** A NULL clear: explicit revert of the override, never a fabricated 0. */
export function buildClearEdit(
	detail: PlanningDetailLike,
	point: PlanningPointLike,
	decision: OverrideDecision
): OverrideEditPayload {
	return {
		run_id: detail.run_id ?? '',
		product_id: detail.product_id,
		location_id: detail.location_id,
		week_start: point.week_start,
		value: null,
		expected_revision: point.override_revision,
		...decidedFields(decision)
	};
}

/** One profile-scoped planner note for the selected series and run context. */
export function buildNoteEdit(detail: PlanningDetailLike, note: string): SeriesNotePayload {
	return {
		run_id: detail.run_id ?? '',
		product_id: detail.product_id,
		location_id: detail.location_id,
		note: note.trim()
	};
}

/** One review event for the selected series and run. */
export function buildReviewEdit(
	detail: PlanningDetailLike,
	action: 'reviewed' | 'reopened'
): ReviewActionPayload {
	return {
		run_id: detail.run_id ?? '',
		product_id: detail.product_id,
		location_id: detail.location_id,
		action
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
