// Cover art holds its blob: URL while on screen, and lets go when it leaves or changes.
import { render, waitFor } from '@testing-library/svelte';

const held: string[] = [];
const released: string[] = [];
vi.mock('$lib/state/app.svelte', () => ({
	api: {
		art: async (p: string) => `blob:${p}`,
		retainArt: (p: string) => {
			held.push(p);
			return () => released.push(p);
		},
	},
}));
const { default: Art } = await import('$lib/components/Art.svelte');

test('art is held while shown and released when it changes or goes', async () => {
	const r = render(Art, { path: 'music/1/cover', eager: true });
	await waitFor(() => expect(r.container.querySelector('img')?.getAttribute('src')).toBe('blob:music/1/cover'));
	expect(held).toEqual(['music/1/cover']);
	expect(released).toEqual([]);
	await r.rerender({ path: 'music/2/cover', eager: true });
	await waitFor(() => expect(r.container.querySelector('img')?.getAttribute('src')).toBe('blob:music/2/cover'));
	expect(released).toEqual(['music/1/cover']);
	r.unmount();
	expect(released).toEqual(['music/1/cover', 'music/2/cover']);
});
