<script lang="ts">
	type Tone = 'neutral' | 'ok' | 'warn' | 'error' | 'info' | 'violet';

	let { label, tone = 'neutral' }: { label: string; tone?: Tone } = $props();
</script>

<span class="chip chip-{tone}">{label}</span>
