import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

// Tauri loads the dev server at http://127.0.0.1:1420 (strict port, no fallback).
export default defineConfig({
	plugins: [
		sveltekit({
			// Desktop shell: static SPA served by Tauri; no server, no SSR.
			preprocess: vitePreprocess(),
			adapter: adapter({ fallback: 'index.html' })
		})
	],
	clearScreen: false,
	server: {
		host: '127.0.0.1',
		port: 1420,
		strictPort: true
	}
});
