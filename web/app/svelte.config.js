import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
export default {
  preprocess: vitePreprocess(),
  kit: {
    // A single static page served by the gateway; all data comes from the /v1 API.
    adapter: adapter({ fallback: 'index.html', strict: true }),
    // Hash routes need no server-side fallback, so the gateway serves plain files only.
    router: { type: 'hash' },
    // Relative asset paths: the app works under any base path.
    paths: { relative: true },
    output: { bundleStrategy: 'split' },
  },
};
