// See https://svelte.dev/docs/kit/types#app.d.ts
declare global {
	namespace App {}
	/** The plugin's release version, from install.xml at build time (vite.config.ts). */
	const __PLUGIN_VERSION__: string;
}
export {};
