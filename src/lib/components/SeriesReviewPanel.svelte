<script lang="ts">
	import type { ReviewActionEdit, SeriesDetail, SeriesNote, SeriesNoteEdit } from '#lib/bindings.ts';
	import { formatTimestamp, shortId } from '#lib/frontend/format.ts';

	let {
		detail,
		onNote,
		onReview,
		onRefresh
	}: {
		detail: SeriesDetail;
		onNote: (edit: SeriesNoteEdit) => Promise<SeriesNote>;
		onReview: (edit: ReviewActionEdit) => Promise<unknown>;
		onRefresh: () => Promise<SeriesDetail>;
	} = $props();

	let draftNote = $state('');
	let busy = $state<'note' | 'review' | null>(null);
	let error = $state<{ code: string; message: string } | null>(null);
	let lastIdentity = '';

	const identity = $derived(`${detail.run_id ?? ''}\u0000${detail.product_id}\u0000${detail.location_id}`);
	const review = $derived(detail.review);
	const reviewed = $derived(review?.status === 'reviewed');
	const canReview = $derived(detail.run_id !== null);

	$effect(() => {
		if (identity !== lastIdentity) {
			lastIdentity = identity;
			draftNote = '';
			busy = null;
			error = null;
		}
	});

	async function submitNote() {
		if (busy !== null || !canReview || draftNote.trim() === '') return;
		busy = 'note';
		error = null;
		try {
			await onNote({
				run_id: detail.run_id ?? '',
				product_id: detail.product_id,
				location_id: detail.location_id,
				note: draftNote
			});
			draftNote = '';
			await onRefresh();
		} catch (failure) {
			error = classify(failure);
		} finally {
			busy = null;
		}
	}

	async function setReview(action: 'reviewed' | 'reopened') {
		if (busy !== null || !canReview) return;
		busy = 'review';
		error = null;
		try {
			await onReview({
				run_id: detail.run_id ?? '',
				product_id: detail.product_id,
				location_id: detail.location_id,
				action
			});
			await onRefresh();
		} catch (failure) {
			error = classify(failure);
		} finally {
			busy = null;
		}
	}

	function classify(failure: unknown): { code: string; message: string } {
		if (failure !== null && typeof failure === 'object') {
			const record = failure as { code?: unknown; message?: unknown };
			const code = typeof record.code === 'string' && record.code !== '' ? record.code : 'unknown_error';
			const message =
				typeof record.message === 'string' && record.message !== ''
					? record.message
					: 'The change could not be saved.';
			return { code, message };
		}
		return { code: 'unknown_error', message: 'The change could not be saved.' };
	}
</script>

<section class="review" aria-labelledby="series-review-heading">
	<div class="review-head">
		<h3 id="series-review-heading">Review &amp; notes</h3>
		{#if canReview}
			<div class="review-state">
				{#if reviewed}
					<span class="chip chip-ok">Reviewed</span>
					{#if review?.reviewed_at}
						<span class="muted">acknowledged {formatTimestamp(review.reviewed_at)}</span>
					{/if}
					<button
						class="btn btn-secondary btn-tiny"
						disabled={busy !== null}
						onclick={() => setReview('reopened')}
					>
						{busy === 'review' ? 'Working…' : 'Reopen'}
					</button>
				{:else}
					<span class="chip chip-neutral">Not reviewed</span>
					<button
						class="btn btn-primary btn-tiny"
						disabled={busy !== null}
						onclick={() => setReview('reviewed')}
					>
						{busy === 'review' ? 'Working…' : 'Mark reviewed'}
					</button>
				{/if}
			</div>
		{:else}
			<span class="muted">Select a forecast run to review this series.</span>
		{/if}
	</div>
	<p class="hint muted">
		A review acknowledges the current evidence for this run. Later override changes or a new run
		reopen it automatically; notes stay with the series across runs.
	</p>

	{#if error}
		<p class="banner banner-error" role="alert">[{error.code}] {error.message}</p>
	{/if}

	{#if canReview}
		<div class="note-composer">
			<label class="field-label" for="series-note-input">Add a note for this series</label>
			<textarea
				id="series-note-input"
				rows="2"
				maxlength="1000"
				placeholder="What should the next review know? (plain text)"
				bind:value={draftNote}
			></textarea>
			<div class="composer-actions">
				<button
					class="btn btn-primary btn-tiny"
					disabled={busy !== null || draftNote.trim() === ''}
					onclick={submitNote}
				>
					{busy === 'note' ? 'Saving…' : 'Add note'}
				</button>
			</div>
		</div>
	{/if}

	{#if detail.notes.length === 0}
		<p class="muted">No notes recorded for this series yet.</p>
	{:else}
		<ul class="notes">
			{#each detail.notes as note (note.id)}
				<li>
					<p class="note-text">{note.note}</p>
					<p class="note-meta muted">
						{formatTimestamp(note.created_at)}
						{#if note.context_run_id}
							· run {shortId(note.context_run_id)}
						{/if}
					</p>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	.review {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	.review h3 {
		margin: 0;
	}

	.review-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.review-state {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.5rem;
		font-size: 0.85rem;
	}

	.hint {
		margin: 0;
		font-size: 0.78rem;
	}

	.note-composer {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
	}

	.note-composer textarea {
		width: 100%;
		resize: vertical;
		font-size: 0.88rem;
	}

	.composer-actions {
		display: flex;
		justify-content: flex-end;
	}

	.notes {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}

	.notes li {
		padding: 0.45rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		background: var(--surface);
	}

	.note-text {
		margin: 0;
		font-size: 0.88rem;
		overflow-wrap: anywhere;
		white-space: pre-wrap;
	}

	.note-meta {
		margin: 0.15rem 0 0;
		font-size: 0.74rem;
	}

	.btn-tiny {
		min-height: 28px;
		padding: 0.2rem 0.55rem;
		font-size: 0.8rem;
	}
</style>
