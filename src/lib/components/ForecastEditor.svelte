<script lang="ts">
	import { onDestroy } from 'svelte';
	import type { OverrideEdit, OverrideRevision, SeriesDetail } from '#lib/bindings.ts';
	import { formatQuantity, overrideReasonLabel } from '#lib/frontend/format.ts';
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
	} from '#lib/frontend/planner.ts';
	import type { PlannerDraftState, PlanningPointLike, SaveFailure } from '#lib/frontend/planner.ts';

	type NoticeTone = 'success' | 'info' | 'warn' | 'error';

	interface DecisionDraft {
		week: string;
		kind: 'save' | 'clear';
		reason: string;
		note: string;
		reviewDate: string;
	}

	const REASON_OPTIONS = [
		['promotion', 'Promotion'],
		['customer_information', 'Customer information'],
		['supply_constraint', 'Supply constraint'],
		['data_quality', 'Data quality'],
		['other', 'Other (note required)']
	] as const;

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
	let decision = $state<DecisionDraft | null>(null);
	// Plain tracking variables: changing them must not trigger reactivity.
	let lastIdentity = '';
	let lastReportedDirty = false;

	// Drafts survive same-identity refreshes untouched; only an actual run,
	// product or location change resets them (and every message with them).
	const currentDraftState = $derived(syncDraftState(draftState, detail));
	const activeDrafts = $derived(currentDraftState.drafts);
	const dirty = $derived(isPlannerDirty(currentDraftState, detail));
	const conflictWeeks = $derived(Object.keys(conflicts).length);
	const today = localToday();
	onDestroy(() => onDirtyChange(false));

	function localToday(): string {
		const now = new Date();
		const month = String(now.getMonth() + 1).padStart(2, '0');
		const day = String(now.getDate()).padStart(2, '0');
		return `${now.getFullYear()}-${month}-${day}`;
	}

	function reviewDue(point: PlanningPointLike): boolean {
		return (
			point.override_value !== null &&
			typeof point.override_review_date === 'string' &&
			point.override_review_date <= today
		);
	}

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
			decision = null;
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
		decision = null;
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
			requestSave(point);
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

	/** Saving a value needs an explicit reason; open the decision form first. */
	function requestSave(point: PlanningPointLike) {
		if (busy !== null || refreshing || !detail.can_override) return;
		const draft = currentDraftState.drafts[point.week_start];
		if (!draft || !isDraftDirty(currentDraftState, point)) return;
		decision = { week: point.week_start, kind: 'save', reason: '', note: '', reviewDate: '' };
	}

	/** Clearing an override is a decision too: reason required, note optional. */
	function requestClear(point: PlanningPointLike) {
		if (busy !== null || refreshing || !detail.can_override || point.override_value === null) return;
		decision = { week: point.week_start, kind: 'clear', reason: '', note: '', reviewDate: '' };
	}

	function cancelDecision() {
		decision = null;
	}

	async function commitDecision(point: PlanningPointLike) {
		const active = decision;
		if (
			!active ||
			active.week !== point.week_start ||
			active.reason === '' ||
			busy !== null ||
			refreshing ||
			!detail.can_override
		) {
			return;
		}
		const started = planningIdentity(detail);
		const decisionFields = {
			reasonCode: active.reason,
			note: active.note,
			reviewDate: active.reviewDate
		};
		let payload: OverrideEdit;
		if (active.kind === 'save') {
			const draft = currentDraftState.drafts[point.week_start];
			if (!draft || !isDraftDirty(currentDraftState, point)) {
				decision = null;
				return;
			}
			payload = buildOverrideEdit(detail, point, draft.raw, draft.expectedRevision, decisionFields);
		} else {
			payload = buildClearEdit(detail, point, decisionFields);
		}
		decision = null;
		busy = { week: point.week_start, kind: active.kind === 'save' ? 'save' : 'revert' };
		clearRowError(point.week_start);
		try {
			const result = await onSave(payload);
			let refreshed = true;
			try {
				await onRefresh();
			} catch {
				refreshed = false;
			}
			if (planningIdentity(detail) !== started) return;
			if (active.kind === 'save') {
				draftState = refreshed
					? dropDraft(draftState, point.week_start)
					: rebaseDraft(draftState, point.week_start, result.revision);
			} else {
				draftState = rebaseDraft(draftState, point.week_start, result.revision);
			}
			clearConflict(point.week_start);
			const label = active.kind === 'save' ? 'Saved' : 'Override cleared for';
			notice = refreshed
				? { tone: 'success', text: `${label} ${point.week_start} as revision ${result.revision}.` }
				: {
						tone: 'warn',
						text: `${label} ${point.week_start} as revision ${result.revision}, but the plan could not be refreshed.`,
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
						{@const activeDecision = decision?.week === point.week_start ? decision : null}
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
								{#if point.override_revision > 0}
									<span class="decision-meta muted">
										{overrideReasonLabel(point.override_reason)}
										{#if point.override_note}· {point.override_note}{/if}
										{#if point.override_review_date}· Review {point.override_review_date}{/if}
										{#if reviewDue(point)}
											<span class="chip chip-warn due-chip">Review due</span>
										{/if}
									</span>
								{/if}
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
										onclick={() => requestSave(point)}
									>
										Save
									</button>
									<button
										class="btn btn-secondary btn-tiny"
										disabled={!detail.can_override || pendingKind !== null || busy !== null || refreshing || point.override_value === null}
										aria-label={`Revert override for week ${point.week_start}`}
										onclick={() => requestClear(point)}
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
						{#if activeDecision}
							<tr class="decision-row">
								<td colspan="5">
									<div class="decision-form">
										<p class="decision-heading">
											{activeDecision.kind === 'save'
												? `Save override for ${point.week_start}`
												: `Clear override for ${point.week_start}`}
											<span class="muted">— a reason is required; it stays with this revision.</span>
										</p>
										<div class="decision-fields">
											<label>
												<span class="field-label">Reason</span>
												<select
													class="decision-reason"
													aria-label={`Reason for week ${point.week_start}`}
													bind:value={activeDecision.reason}
												>
													<option value="">Choose a reason…</option>
													{#each REASON_OPTIONS as [code, label] (code)}
														<option value={code}>{label}</option>
													{/each}
												</select>
											</label>
											<label>
												<span class="field-label">
													Note {activeDecision.reason === 'other' ? '(required)' : '(optional)'}
												</span>
												<input
													type="text"
													class="decision-note"
													maxlength="1000"
													placeholder="Plain text, stored with the revision"
													aria-label={`Note for week ${point.week_start}`}
													bind:value={activeDecision.note}
												/>
											</label>
											<label>
												<span class="field-label">Review date (optional)</span>
												<input
													type="date"
													class="decision-date"
													aria-label={`Review date for week ${point.week_start}`}
													bind:value={activeDecision.reviewDate}
												/>
											</label>
											<div class="decision-actions">
												<button
													class="btn btn-primary btn-tiny"
													disabled={activeDecision.reason === '' ||
														(activeDecision.reason === 'other' && activeDecision.note.trim() === '') ||
														busy !== null ||
														refreshing}
													onclick={() => commitDecision(point)}
												>
													{activeDecision.kind === 'save' ? 'Save revision' : 'Clear override'}
												</button>
												<button class="btn btn-secondary btn-tiny" onclick={cancelDecision}>
													Cancel
												</button>
											</div>
										</div>
									</div>
								</td>
							</tr>
						{/if}
					{/each}
				</tbody>
			</table>
		</div>

		<p class="hint muted">
			Enter opens the save form for the week, Escape discards that week's draft. Saving or clearing
			a week records a reason (and optional note and review date) with the new revision. Revert
			clears a saved override; without a baseline the week becomes Missing.
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

	.decision-meta {
		display: block;
		margin-top: 0.2rem;
		max-width: 22rem;
		font-size: 0.74rem;
		overflow-wrap: anywhere;
	}

	.due-chip {
		margin-left: 0.3rem;
	}

	.decision-row td {
		background: var(--surface-subtle);
		padding: 0.45rem 0.55rem;
	}

	.decision-form {
		display: flex;
		flex-direction: column;
		gap: 0.45rem;
	}

	.decision-heading {
		margin: 0;
		font-size: 0.85rem;
		font-weight: 600;
	}

	.decision-heading .muted {
		font-weight: 400;
	}

	.decision-fields {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-end;
		gap: 0.6rem;
	}

	.decision-fields label {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	.decision-reason {
		min-width: 12rem;
	}

	.decision-note {
		min-width: 16rem;
	}

	.decision-actions {
		display: flex;
		gap: 0.35rem;
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
