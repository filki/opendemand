<script lang="ts">
	import type { JobInfo } from '$lib/bindings';
	import StatusChip from '$lib/components/StatusChip.svelte';
	import { shortId } from '$lib/frontend/format';
	import {
		isActiveJobStatus,
		jobKindLabel,
		jobStatusLabel,
		progressPercent
	} from '$lib/frontend/jobs';

	let {
		job,
		notice,
		recentJobs,
		onCancel,
		onDismiss,
		onRefresh
	}: {
		job: JobInfo | null;
		notice: string | null;
		recentJobs: JobInfo[];
		onCancel: () => void;
		onDismiss: () => void;
		onRefresh: () => void;
	} = $props();

	function jobTone(status: string): 'neutral' | 'ok' | 'warn' | 'error' | 'info' {
		switch (status.toLowerCase()) {
			case 'completed':
				return 'ok';
			case 'failed':
				return 'error';
			case 'cancelled':
			case 'canceled':
				return 'neutral';
			default:
				return 'info';
		}
	}

	const cancelling = $derived(
		job !== null && (job.status === 'cancelling' || job.status === 'canceling')
	);
</script>

<section class="panel section" aria-labelledby="job-heading">
	<h2 id="job-heading">Job</h2>

	{#if job && isActiveJobStatus(job.status)}
		<div class="job">
			<div class="job-top">
				<strong>{jobKindLabel(job.kind)}</strong>
				<StatusChip label={jobStatusLabel(job.status)} tone={jobTone(job.status)} />
			</div>
			<p class="phase muted" role="status">
				{job.phase ? `${jobStatusLabel(job.status)} — ${job.phase}` : jobStatusLabel(job.status)}
			</p>
			<div
				class="progress"
				role="progressbar"
				aria-label="{jobKindLabel(job.kind)} progress"
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={progressPercent(job.progress)}
			>
				<div class="progress-fill" style="width: {progressPercent(job.progress)}%"></div>
			</div>
			<div class="job-actions">
				<button class="btn btn-danger" onclick={onCancel} disabled={cancelling}>
					{cancelling ? 'Cancelling…' : 'Cancel job'}
				</button>
			</div>
		</div>
	{:else if job}
		<div class="job">
			<div class="job-top">
				<strong>{jobKindLabel(job.kind)}</strong>
				<StatusChip label={jobStatusLabel(job.status)} tone={jobTone(job.status)} />
			</div>
			{#if notice}
				<p class="notice" role="status">{notice}</p>
			{/if}
			{#if job.error}
				<p class="banner banner-error error" role="alert">
					[{job.error.code}] {job.error.message}
				</p>
			{/if}
			<div class="job-actions">
				<button class="btn btn-secondary" onclick={onDismiss}>Dismiss</button>
			</div>
		</div>
	{:else if notice}
		<p class="banner banner-success" role="status">{notice}</p>
	{/if}

	{#if recentJobs.length > 0}
		<h3 class="recent-heading" id="recent-heading">Recent jobs</h3>
		<ul class="recent" aria-labelledby="recent-heading">
			{#each recentJobs as recent (recent.id)}
				<li>
					<span class="recent-kind">{jobKindLabel(recent.kind)}</span>
					<StatusChip label={jobStatusLabel(recent.status)} tone={jobTone(recent.status)} />
					<span class="mono muted" title={recent.id}>{shortId(recent.id, 10)}</span>
					{#if recent.error}
						<span class="recent-error" title={recent.error.message}>{recent.error.code}</span>
					{/if}
				</li>
			{/each}
		</ul>
		<button class="btn btn-secondary btn-small" onclick={onRefresh}>Refresh list</button>
	{/if}
</section>

<style>
	.section {
		padding: 1rem 1.1rem;
	}

	h2 {
		margin-bottom: 0.6rem;
	}

	.job-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.phase {
		margin: 0.35rem 0 0.5rem;
		font-size: 0.88rem;
	}

	.notice {
		margin: 0.5rem 0 0;
		font-size: 0.88rem;
	}

	.progress {
		height: 8px;
		border-radius: 999px;
		background: #e7ebf2;
		overflow: hidden;
	}

	.progress-fill {
		height: 100%;
		background: var(--cobalt);
		border-radius: 999px;
	}

	.error {
		margin-top: 0.6rem;
	}

	.job-actions {
		margin-top: 0.6rem;
	}

	.recent-heading {
		margin: 1rem 0 0.4rem;
	}

	.recent {
		margin: 0 0 0.6rem;
		padding: 0;
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		font-size: 0.85rem;
	}

	.recent li {
		display: flex;
		align-items: center;
		gap: 0.45rem;
		flex-wrap: wrap;
	}

	.recent-kind {
		font-weight: 600;
	}

	.recent-error {
		color: var(--danger);
		font-family: var(--font-mono);
		font-size: 0.8em;
	}

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
	}
</style>
