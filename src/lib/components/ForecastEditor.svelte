<script lang="ts">
	import { onDestroy } from 'svelte';
	import type { OverrideEdit, OverrideRevision, SeriesDetail } from '$lib/bindings';
	import { formatQuantity } from '$lib/frontend/format';
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
	} from '$lib/frontend/planner';
	import type { PlannerDraftState, PlanningPointLike, SaveFailure } from '$lib/frontend/planner';

	type NoticeTone = 'success' | 'info' | 'warn' | 'error';

	let {
		detail,
		onSave,
		onRefresh,
		onDirtyChange
	}: {
		detail: SeriesDetail;
		onSave: (edit: OverrideEdit) => Promise<OverrideRevision>;
		onRefresh: () => Promise<SeriesDetail>;
		onDirtyChange: (dirty: boolean) => void;
	} = $props();

	// The state starts unbound; `currentDraftState` and the effect below adopt
	// the live identity before any draft can exist for it.
	let draftState = $state<PlannerDraftState>({ identity: '', drafts: {} });
	let busy = $state<{ week: string; kind: 'save' | 'revert' } | null>(null);
	let refreshing = $state(false);
	let rowErrors = $state<Record<string, SaveFailure>>({});
	let conflicts = $state<Record<string, true>>({});
	let notice = $state<{ tone: NoticeTone; text: string; canRefresh?: boolean } | null>(null);
	// Plain tracking variables: changing them must not trigger reactivity.
	let lastIdentity = '';
	let lastReportedDirty = false;

	// Drafts survive same-identity refreshes untouched; only an actual run,
	// product or location change resets them (and every message with them).
	const currentDraftState = $derived(syncDraftState(draftState, detail));
	const activeDrafts = $derived(currentDraftState.drafts);
	const dirty = $derived(isPlannerDirty(currentDraftState, detail));
	const conflictWeeks = $derived(Object.keys(conflicts).length);
	onDestroy(() => onDirtyChange(false));

	$effect(() => {
		const identity = planningIdentity(detail);
		if (identity !== lastIdentity) {
			lastIdentity = identity;
			draftState = createDraftState(detail);
			busy = null;
			refreshing = false;
			rowErrors = {};
			conflicts = {};
			notice = null;
		}
	});

	$effect(() => {
		if (dirty !== lastReportedDirty) {
			lastReportedDirty = dirty;
			onDirtyChange(dirty);
		}
	});

	function errorId(week: string): string {
		return `forecast-editor-error-${week}`;
	}

	function noticeClass(tone: NoticeTone): string {
		return `banner-${tone}`;
	}

	function clearRowError(week: string) {
		if (rowErrors[week]) {
			const next = { ...rowErrors };
			delete next[week];
			rowErrors = next;
		}
	}

	function clearConflict(week: string) {
		if (conflicts[week]) {
			const next = { ...conflicts };
			delete next[week];
			conflicts = next;
		}
	}

	function discardDrafts() {
		if (busy !== null || refreshing) return;
		draftState = createDraftState(detail);
		rowErrors = {};
		conflicts = {};
		notice = { tone: 'info', text: 'Unsaved edits discarded. Saved overrides are unchanged.' };
	}

	function handleInput(point: PlanningPointLike, raw: string) {
		const existing = currentDraftState.drafts[point.week_start];
		draftState = setDraft(
			currentDraftState,
			point.week_start,
			raw,
			existing ? existing.expectedRevision : point.override_revision
		);
		clearRowError(point.week_start);
	}

	function handleKeydown(event: KeyboardEvent, point: PlanningPointLike) {
		if (event.isComposing) return;
		if (event.key === 'Enter') {
			event.preventDefault();
			save(point);
		} else if (event.key === 'Escape') {
			event.preventDefault();
			draftState = dropDraft(currentDraftState, point.week_start);
			clearRowError(point.week_start);
			clearConflict(point.week_start);
		}
	}

	async function refreshPlan(rebase: boolean) {
		if (refreshing || busy !== null) return;
		const started = planningIdentity(detail);
		refreshing = true;
		try {
			const fresh = await onRefresh();
			if (planningIdentity(detail) !== started) return;
			if (rebase) {
				draftState = rebaseDrafts(currentDraftState, fresh);
				const remaining: Record<string, SaveFailure> = {};
				for (const [week, failure] of Object.entries(rowErrors)) {
					if (!failure.conflict) remaining[week] = failure;
				}
				rowErrors = remaining;
				conflicts = {};
				notice = { tone: 'info', text: 'Plan refreshed. Review your edited values and save again.' };
			} else {
				notice = { tone: 'info', text: 'Plan refreshed.' };
			}
		} catch (error) {
			if (planningIdentity(detail) !== started) return;
			notice = {
				tone: 'error',
				text: `The plan could not be refreshed: ${classifySaveFailure(error).message}`
			};
		} finally {
			refreshing = false;
		}
	}

	async function save(point: PlanningPointLike) {
		if (busy !== null || refreshing || !detail.can_override) return;
		const draft = currentDraftState.drafts[point.week_start];
		if (!draft || !isDraftDirty(currentDraftState, point)) return;
		const started = planningIdentity(detail);
		busy = { week: point.week_start, kind: 'save' };
		clearRowError(point.week_start);
		try {
			const result = await onSave(
				buildOverrideEdit(detail, point, draft.raw, draft.expectedRevision)
			);
			let refreshed = true;
			try {
				await onRefresh();
			} catch {
				refreshed = false;
			}
			if (planningIdentity(detail) !== started) return;
			draftState = refreshed
				? dropDraft(draftState, point.week_start)
				: rebaseDraft(draftState, point.week_start, result.revision);
			clearConflict(point.week_start);
			notice = refreshed
				? { tone: 'success', text: `Saved ${point.week_start} as revision ${result.revision}.` }
				: {
						tone: 'warn',
						text: `Saved ${point.week_start} as revision ${result.revision}, but the plan could not be refreshed.`,
						canRefresh: true
					};
		} catch (error) {
			if (planningIdentity(detail) !== started) return;
			const failure = classifySaveFailure(error);
			rowErrors = { ...rowErrors, [point.week_start]: failure };
			if (failure.conflict) conflicts = { ...conflicts, [point.week_start]: true };
		} finally {
			if (planningIdentity(detail) === started) busy = null;
		}
	}

	async function revert(point: PlanningPointLike) {
		if (busy !== null || refreshing || !detail.can_override || point.override_value === null) return;
		const started = planningIdentity(detail);
		busy = { week: point.week_start, kind: 'revert' };
		clearRowError(point.week_start);
		try {
			const result = await onSave(buildClearEdit(detail, point));
			let refreshed = true;
			try {
				await onRefresh();
			} catch {
				refreshed = false;
			}
			if (planningIdentity(detail) !== started) return;
			draftState = rebaseDraft(draftState, point.week_start, result.revision);
			clearConflict(point.week_start);
			notice = refreshed
				? { tone: 'success', text: `Override cleared for ${point.week_start} (revision ${result.revision}).` }
				: {
						tone: 'warn',
						text: `Override cleared for ${point.week_start} (revision ${result.revision}), but the plan could not be refreshed.`,
						canRefresh: true
					};
		} catch (error) {
			if (planningIdentity(detail) !== started) return;
			const failure = classifySaveFailure(error);
			rowErrors = { ...rowErrors, [point.week_start]: failure };
			if (failure.conflict) conflicts = { ...conflicts, [point.week_start]: true };
		} finally {
			if (planningIdentity(detail) === started) busy = null;
		}
	}
</script>

<section class="editor" aria-labelledby="forecast-editor-heading">
	<div class="editor-heading">
		<h3 id="forecast-editor-heading">Weekly plan</h3>
		{#if dirty}
			<button class="btn btn-secondary btn-tiny" disabled={busy !== null || refreshing} onclick={discardDrafts}>Discard unsaved edits</button>
		{/if}
	</div>

	{#if detail.planning.length === 0}
		<p class="muted">
			{detail.run_id === null
				? 'Select a forecast run to plan weekly overrides.'
				: 'No planning weeks are available for this series.'}
		</p>
	{:else}
		{#if !detail.can_override}
			<p class="banner banner-warn">This run is not completed, so weekly overrides are read-only.</p>
		{/if}

		{#if notice}
			<div class="banner {noticeClass(notice.tone)}">
				<p role={notice.tone === 'error' ? 'alert' : 'status'}>{notice.text}</p>
				{#if notice.canRefresh}
					<button
						class="btn btn-secondary btn-tiny"
						disabled={refreshing || busy !== null}
						onclick={() => refreshPlan(false)}
					>
						{refreshing ? 'Refreshing…' : 'Refresh plan'}
					</button>
				{/if}
			</div>
		{/if}

		{#if conflictWeeks > 0}
			<div class="banner banner-warn">
				<p role="alert">
					{conflictWeeks === 1 ? 'One edited week changed' : `${conflictWeeks} edited weeks changed`}
					since it was loaded. Refresh the plan, review your values, and save again.
				</p>
				<button
					class="btn btn-secondary btn-tiny"
					disabled={refreshing || busy !== null}
					onclick={() => refreshPlan(true)}
				>
					{refreshing ? 'Refreshing…' : 'Refresh plan & review'}
				</button>
			</div>
		{/if}

		<div class="table-wrap">
			<table class="planner">
				<caption class="sr-only">Weekly plan for {detail.series_key} — edit overrides per week.</caption>
				<thead>
					<tr>
						<th scope="col">Week</th>
						<th scope="col">Baseline</th>
						<th scope="col">Override</th>
						<th scope="col">Effective</th>
						<th scope="col">Actions</th>
					</tr>
				</thead>
				<tbody>
					{#each detail.planning as point (point.week_start)}
						{@const draft = activeDrafts[point.week_start]}
						{@const rowDirty = isDraftDirty(currentDraftState, point)}
						{@const rowError = rowErrors[point.week_start]}
						{@const pendingKind =
							busy !== null && busy.week === point.week_start ? busy.kind : null}
						<tr>
							<th scope="row" class="week mono">
								{point.week_start}
								<span class="muted horizon">H{point.horizon}</span>
							</th>
							<td class="num">{formatQuantity(point.baseline)}</td>
							<td class="override-cell">
								<input
									type="text"
									class="cell-input"
									class:overridden={point.override_value !== null}
									class:draft={rowDirty}
									value={draft ? draft.raw : authoritativeInput(point.override_value)}
									placeholder="—"
									disabled={!detail.can_override}
									readonly={pendingKind !== null}
									inputmode="decimal"
									autocomplete="off"
									spellcheck="false"
									aria-label={`Override for week ${point.week_start}`}
									aria-describedby={rowError ? errorId(point.week_start) : undefined}
									aria-invalid={rowError ? 'true' : undefined}
									oninput={(event) => handleInput(point, event.currentTarget.value)}
									onkeydown={(event) => handleKeydown(event, point)}
								/>
								{#if rowError}
									<span class="cell-error" id={errorId(point.week_start)}>
										[{rowError.code}] {rowError.message}
									</span>
								{/if}
							</td>
							<td class="num effective-cell">
								{#if point.effective === null}
									<span class="muted">Missing</span>
								{:else}
									{formatQuantity(point.effective)}
									{#if point.value_source === 'override'}
										<span class="chip chip-violet override-chip">Override</span>
									{/if}
								{/if}
							</td>
							<td class="actions-cell">
								<div class="row-actions">
									<button
										class="btn btn-primary btn-tiny"
										disabled={!detail.can_override || pendingKind !== null || busy !== null || refreshing || !rowDirty}
										aria-label={`Save override for week ${point.week_start}`}
										onclick={() => save(point)}
									>
										Save
									</button>
									<button
										class="btn btn-secondary btn-tiny"
										disabled={!detail.can_override || pendingKind !== null || busy !== null || refreshing || point.override_value === null}
										aria-label={`Revert override for week ${point.week_start}`}
										onclick={() => revert(point)}
									>
										Revert
									</button>
									{#if pendingKind}
										<span class="pending muted" role="status">
											{pendingKind === 'save' ? 'Saving…' : 'Clearing…'}
										</span>
									{/if}
								</div>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>

		<p class="hint muted">
			Enter saves the week, Escape discards that week's draft. Revert clears a saved override;
			without a baseline the week becomes Missing.
		</p>
	{/if}
</section>

<style>
	.editor {
		display: flex;
		flex-direction: column;
		gap: 0.6rem;
	}

	.editor h3 {
		margin: 0;
	}

	.editor-heading {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.banner {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.banner p {
		margin: 0;
	}

	.table-wrap {
		overflow-x: auto;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
	}

	.planner {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.85rem;
	}

	.planner th,
	.planner td {
		padding: 0.3rem 0.55rem;
		text-align: left;
		border-bottom: 1px solid var(--border);
		vertical-align: middle;
	}

	.planner tbody tr:last-child th,
	.planner tbody tr:last-child td {
		border-bottom: 0;
	}

	.planner thead th {
		background: var(--surface-subtle);
		color: var(--ink-soft);
		font-size: 0.78rem;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		white-space: nowrap;
	}

	.num {
		font-variant-numeric: tabular-nums;
	}

	.week {
		white-space: nowrap;
		font-weight: 600;
	}

	.horizon {
		font-size: 0.72rem;
		font-weight: 400;
		margin-left: 0.4rem;
	}

	.override-cell {
		min-width: 9rem;
	}

	.cell-input {
		width: 100%;
		min-width: 6rem;
		padding: 0.28rem 0.45rem;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}

	.cell-input.overridden {
		border-color: var(--violet);
	}

	.cell-input.draft {
		border-color: var(--cobalt);
	}

	.cell-input[aria-invalid='true'] {
		border-color: var(--danger);
	}

	.cell-input:read-only,
	.cell-input:disabled {
		background: var(--surface-subtle);
		color: var(--ink-muted);
	}

	.cell-error {
		display: block;
		margin-top: 0.15rem;
		max-width: 18rem;
		font-size: 0.74rem;
		color: var(--danger);
	}

	.effective-cell {
		white-space: nowrap;
	}

	.override-chip {
		margin-left: 0.35rem;
	}

	.actions-cell {
		white-space: nowrap;
	}

	.row-actions {
		display: flex;
		align-items: center;
		gap: 0.35rem;
	}

	.btn-tiny {
		min-height: 28px;
		padding: 0.2rem 0.55rem;
		font-size: 0.8rem;
	}

	.pending {
		font-size: 0.78rem;
	}

	.hint {
		margin: 0;
		font-size: 0.78rem;
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
		border: 0;
	}
</style>
