// In-app navigation goes through SvelteKit's router (never `location.hash = …`, which reloads).
import { goto } from '$app/navigation';
import { go, href } from '#lib/nav';

test('routes become hash links', () => {
	expect(href('/')).toBe('#/');
	expect(href('/playlist/3')).toBe('#/playlist/3');
	expect(href('playlists')).toBe('#/playlists');
	expect(href('#/search?q=x')).toBe('#/search?q=x');
});

test('go navigates through the router', async () => {
	vi.mocked(goto).mockClear();
	await go('/playlists', { replaceState: true });
	expect(goto).toHaveBeenCalledWith('#/playlists', { replaceState: true });
});
