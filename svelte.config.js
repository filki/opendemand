import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	preprocess: vitePreprocess(),
	kit: {
		// Desktop shell: static SPA served by Tauri; no server, no SSR.
		adapter: adapter({ fallback: 'index.html' })
	}
};

export default config;
