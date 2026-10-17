<script lang="ts">
	import type { JobInfo } from '#lib/bindings.ts';
	import StatusChip from '#lib/components/StatusChip.svelte';
	import {
		isActiveJobStatus,
		jobKindLabel,
		jobPhaseLabel,
		jobStatusLabel,
		progressPercent
	} from '#lib/frontend/jobs.ts';

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
	const percent = $derived(job ? progressPercent(job.progress) : 0);
</script>

<section class="panel section" aria-labelledby="activity-heading">
	<header class="heading">
		<h2 id="activity-heading">Activity</h2>
		{#if job}
			<StatusChip label={jobStatusLabel(job.status)} tone={jobTone(job.status)} />
		{/if}
	</header>

	{#if job && isActiveJobStatus(job.status)}
		<div class="job">
			<strong class="job-kind">{jobKindLabel(job.kind)}</strong>
			<p class="phase muted" role="status">{jobPhaseLabel(job.phase)}. {percent}% complete.</p>
			<div
				class="progress"
				role="progressbar"
				aria-label="{jobKindLabel(job.kind)} progress"
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={percent}
				aria-valuetext="{percent}% complete"
			>
				<div class="progress-fill" style="width: {percent}%"></div>
			</div>
			<div class="job-actions">
				<button class="btn btn-danger" onclick={onCancel} disabled={cancelling}>
					{cancelling ? 'Cancelling…' : 'Cancel task'}
				</button>
			</div>
		</div>
	{:else if job}
		<div class="job">
			<strong class="job-kind">{jobKindLabel(job.kind)}</strong>
			{#if notice}
				<p class="notice" role="status">{notice}</p>
			{/if}
			{#if job.error}
				<div class="banner banner-error error" role="alert">
					<p>{job.error.message}</p>
					<details>
						<summary>Technical details</summary>
						<code>{job.error.code}</code>
					</details>
				</div>
			{/if}
			<div class="job-actions">
				<button class="btn btn-secondary" onclick={onDismiss}>Clear message</button>
			</div>
		</div>
	{:else if notice}
		<p class="banner banner-success" role="status">{notice}</p>
	{/if}

	{#if recentJobs.length > 0}
		<details class="recent-details">
			<summary>Recent activity ({recentJobs.length})</summary>
			<ul class="recent">
				{#each recentJobs as recent (recent.id)}
					<li>
						<div class="recent-row">
							<strong>{jobKindLabel(recent.kind)}</strong>
							<StatusChip label={jobStatusLabel(recent.status)} tone={jobTone(recent.status)} />
						</div>
						{#if recent.error}
							<details class="recent-error">
								<summary>{recent.error.message}</summary>
								<code>{recent.error.code}</code>
							</details>
						{/if}
					</li>
				{/each}
			</ul>
			<button class="btn btn-secondary btn-small" onclick={onRefresh}>Refresh activity</button>
		</details>
	{/if}
</section>

<style>
	.section {
		padding: 0.9rem 1.1rem;
	}

	.heading {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.75rem;
	}

	h2 {
		margin: 0;
	}

	.job {
		margin-top: 0.75rem;
	}

	.job-kind {
		font-size: 0.95rem;
	}

	.phase {
		margin: 0.25rem 0 0.55rem;
		font-size: 0.88rem;
	}

	.notice {
		max-width: 80ch;
		margin: 0.4rem 0 0;
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

	.job-actions {
		display: flex;
		justify-content: flex-end;
		margin-top: 0.6rem;
	}

	.error {
		margin-top: 0.6rem;
	}

	.error p {
		margin: 0 0 0.35rem;
	}

	.error summary,
	.recent-details summary {
		width: fit-content;
		color: var(--ink-soft);
		font-size: 0.84rem;
		font-weight: 600;
		cursor: pointer;
	}

	.recent-details {
		margin-top: 0.75rem;
		padding-top: 0.65rem;
		border-top: 1px solid var(--border);
	}

	.recent {
		margin: 0.55rem 0;
		padding: 0;
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 0.45rem;
		font-size: 0.85rem;
	}

	.recent li {
		min-width: 0;
		padding-bottom: 0.45rem;
		border-bottom: 1px solid var(--border);
	}

	.recent-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.recent-error {
		margin-top: 0.3rem;
		font-size: 0.82rem;
	}

	.recent-error summary {
		max-width: 100%;
		font-weight: 400;
		overflow-wrap: anywhere;
	}

	.recent-error code,
	.error code {
		display: block;
		margin-top: 0.3rem;
		font-family: var(--font-mono);
		font-size: 0.8rem;
	}

	.btn-small {
		min-height: 32px;
		padding: 0.25rem 0.6rem;
		font-size: 0.85rem;
	}
</style>
