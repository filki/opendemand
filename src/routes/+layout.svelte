<script lang="ts">
	import { QueryClient, QueryClientProvider } from '@tanstack/svelte-query';
	import type { Snippet } from 'svelte';
	import '$lib/frontend/styles.css';

	let { children }: { children?: Snippet } = $props();

	const queryClient = new QueryClient({
		defaultOptions: {
			queries: {
				retry: 0,
				refetchOnWindowFocus: false
			}
		}
	});
</script>

<a class="skip-link" href="#main">Skip to main content</a>

<QueryClientProvider client={queryClient}>
	{@render children?.()}
</QueryClientProvider>
