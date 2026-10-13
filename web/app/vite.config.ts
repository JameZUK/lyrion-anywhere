import { sveltekit } from '@sveltejs/kit/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit(), svelteTesting()],
  server: {
    // Development: proxy the API to a local gateway (see README).
    proxy: {
      '/v1': { target: process.env.ANYWHERE_GATEWAY ?? 'http://127.0.0.1:9443', ws: true },
      '/healthz': process.env.ANYWHERE_GATEWAY ?? 'http://127.0.0.1:9443',
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['tests/**/*.test.ts'],
    setupFiles: ['tests/setup.ts'],
  },
});
