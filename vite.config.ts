import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Tauri loads the dev server at http://127.0.0.1:1420 (strict port, no fallback).
export default defineConfig({
	plugins: [sveltekit()],
	clearScreen: false,
	server: {
		host: '127.0.0.1',
		port: 1420,
		strictPort: true
	}
});
