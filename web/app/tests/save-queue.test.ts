// "Save as playlist" on the queue: the name rules (as the gateway's), the request and its
// answers, and the dialog: admins only, only when the gateway offers it, never replacing a
// playlist without asking, and a message with a link to the saved playlist.
import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { goto } from '$app/navigation';
import { ApiError } from '$lib/api/http';
import { cleanPlaylistName, playlistNameProblem, suggestPlaylistName, PLAYLIST_NAME_MAX } from '$lib/queue/save-name';

const P = '02:4c:41:00:00:01';

// The dialog's gateway (the request itself is tested with MeApi below).
const fakeMe = vi.hoisted(() => ({
	sent: [] as { player: string; name: string; replace: boolean }[],
	answer: (_name: string, _replace: boolean): unknown => ({}),
	feats: { save_queue: true } as Record<string, boolean>,
}));
vi.mock('$lib/state/app.svelte', async () => await import('./helpers/fake-app.svelte'));
vi.mock('$lib/state/me', () => ({
	me: {
		features: async () => fakeMe.feats,
		saveQueue: async (player: string, name: string, replace = false) => {
			fakeMe.sent.push({ player, name, replace });
			const a = fakeMe.answer(name, replace);
			if (a instanceof Error) throw a;
			return a;
		},
	},
}));

// --- the name ------------------------------------------------------------------------------

test('good names pass, cleaned to NFC and trimmed', () => {
	for (const n of ['Road trip', 'Sigur Rós – live', '日本語', 'Mix vol. 2', 'AC-DC & friends (2026)', 'Console', 'é'.repeat(100)]) {
		expect(playlistNameProblem(cleanPlaylistName(n)), n).toBe('');
	}
	expect(cleanPlaylistName('  Cafe\u0301 \t')).toBe('Café');
	expect(cleanPlaylistName('Cafe\u0301')).toHaveLength(4);
});

test('bad names are refused with a reason', () => {
	for (const n of [
		'',
		'..',
		'../etc/passwd',
		'a..b',
		'a/b',
		'AC/DC',
		'a\\b',
		'C:\\x',
		'name:x',
		'a*b',
		'why?',
		'say "hi"',
		'<b>',
		'a|b',
		'.hidden',
		'trailing.',
		'new\nline',
		'tab\there',
		'nul\u0000',
		'esc\u001b[2J',
		'del\u007f',
		'bidi\u202etxt',
		'zero\u200bwidth',
		'bom\ufeff',
		'CON',
		'nul.txt',
		'x'.repeat(PLAYLIST_NAME_MAX + 1),
	]) {
		expect(playlistNameProblem(n), JSON.stringify(n)).not.toBe('');
	}
});

test('the name check never throws, and what it accepts keeps every rule', () => {
	let seed = 0x5eed;
	const rnd = () => (seed = (seed * 1103515245 + 12345) % 2 ** 31);
	const alphabet = [...'ab Z.é/\\:*?"<>|\u0000\n\t\u007f\u202e\u200b日 ..-_~%CON'];
	for (let i = 0; i < 5000; i++) {
		const len = rnd() % 130;
		let s = '';
		for (let k = 0; k < len; k++) s += rnd() % 5 === 0 ? String.fromCodePoint(rnd() % 0x10ffff) : alphabet[rnd() % alphabet.length];
		const clean = cleanPlaylistName(s);
		if (!playlistNameProblem(clean)) {
			expect(clean.length).toBeGreaterThan(0);
			expect([...clean].length).toBeLessThanOrEqual(PLAYLIST_NAME_MAX);
			expect(clean).not.toMatch(/[/\\:*?"<>|]|\.\.|^\.|\.$/);
		}
	}
});

test('suggested names: the album when the queue is one album, else the date', () => {
	const day = new Date(2026, 9, 14);
	expect(suggestPlaylistName([{ album: 'Kind of Blue' }, { album: 'Kind of Blue' }], day)).toBe('Kind of Blue');
	expect(suggestPlaylistName([{ album: 'A' }, { album: 'B' }], day)).toBe('Queue 14 October 2026');
	expect(suggestPlaylistName([], day)).toBe('Queue 14 October 2026');
	// An album name that isn't a usable file name is tidied, or not used.
	expect(suggestPlaylistName([{ album: 'Live 1990/91: Part 2?' }], day)).toBe('Live 1990 91 Part 2');
	expect(suggestPlaylistName([{ album: '...' }], day)).toBe('Queue 14 October 2026');
	expect(playlistNameProblem(suggestPlaylistName([{ album: 'x'.repeat(300) }], day))).toBe('');
});

// --- the request -----------------------------------------------------------------------------

describe('MeApi.saveQueue', () => {
	type Call = { url: string; method: string; body: unknown };
	let calls: Call[] = [];
	let reply: { status: number; body: unknown } = { status: 200, body: {} };

	beforeEach(() => {
		calls = [];
		globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
			calls.push({ url: String(url), method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : undefined });
			return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json' } });
		}) as typeof fetch;
	});

	const api = async () => new (await import('$lib/api/me')).MeApi((fn) => fn('tok'));

	test('saved: the player and the cleaned name are sent', async () => {
		reply = { status: 200, body: { id: 12, name: 'Road trip' } };
		const r = await (await api()).saveQueue('02:4C:41:00:00:01', '  Road trip ');
		expect(r).toEqual({ saved: { id: 12, name: 'Road trip' } });
		expect(calls[0]).toEqual({ url: `/v1/players/${encodeURIComponent(P)}/queue/save`, method: 'POST', body: { name: 'Road trip', replace: false } });
		await (await api()).saveQueue(P, 'Road trip', true);
		expect(calls[1]!.body).toEqual({ name: 'Road trip', replace: true });
	});

	test('409: an existing playlist, or an empty queue', async () => {
		reply = { status: 409, body: { error: 'a playlist with that name already exists', exists: { id: 5, name: 'Road Trip' } } };
		expect(await (await api()).saveQueue(P, 'road trip')).toEqual({ exists: { id: 5, name: 'Road Trip' } });
		reply = { status: 409, body: { error: 'the queue is empty' } };
		expect(await (await api()).saveQueue(P, 'x')).toEqual({ empty: true });
	});

	test('refusals and odd answers are errors', async () => {
		reply = { status: 403, body: { error: 'changing playlists needs the admin role' } };
		await expect((await api()).saveQueue(P, 'x')).rejects.toMatchObject({ status: 403, message: 'changing playlists needs the admin role' });
		reply = { status: 400, body: { error: 'names can\u2019t contain ..' } };
		await expect((await api()).saveQueue(P, 'x')).rejects.toBeInstanceOf(ApiError);
		reply = { status: 200, body: { id: 'x', name: 3 } };
		await expect((await api()).saveQueue(P, 'x')).rejects.toBeInstanceOf(ApiError);
	});

	test('bad names and players are refused before sending', async () => {
		const me = await api();
		for (const n of ['', '../x', 'a/b', 'x:y', '.x', 'x'.repeat(101)]) await expect(me.saveQueue(P, n)).rejects.toBeInstanceOf(RangeError);
		for (const p of ['nope', '../x', '02:4c:41:00:00:0g']) await expect(me.saveQueue(p, 'x')).rejects.toBeInstanceOf(RangeError);
		expect(calls).toHaveLength(0);
	});

	test('features: only true flags count', async () => {
		reply = { status: 200, body: { user: {}, features: { save_queue: true, queue_edit: 1, analysis: false, x: 'true' } } };
		expect(await (await api()).features()).toEqual({ save_queue: true });
		expect(calls[0]).toMatchObject({ url: '/v1/me', method: 'GET' });
		reply = { status: 200, body: { user: {} } };
		expect(await (await api()).features()).toEqual({});
	});
});

// --- the dialog ------------------------------------------------------------------------------

describe('Save as playlist', () => {
	const { sent } = fakeMe;
	const answer = (f: (name: string, replace: boolean) => unknown) => (fakeMe.answer = f);

	const tracks = [{ album: 'Kind of Blue' }, { album: 'Kind of Blue' }];

	async function setup(admin = true) {
		const fake = await import('./helpers/fake-app.svelte');
		const { serverFeatures } = await import('$lib/state/features.svelte');
		serverFeatures.reset();
		fake.app.isAdmin = admin;
		fake.app.playerId = P;
		const { default: SaveQueue } = await import('../src/lib/components/SaveQueue.svelte');
		const r = render(SaveQueue, { tracks });
		for (let i = 0; i < 5; i++) {
			await tick();
			await new Promise((res) => setTimeout(res, 0));
		}
		return r;
	}

	beforeEach(() => {
		sent.length = 0;
		fakeMe.feats = { save_queue: true };
		answer((name) => ({ saved: { id: 9, name } }));
		vi.mocked(goto).mockClear();
	});

	test('listeners, and gateways without the feature, see no button', async () => {
		const r = await setup(false);
		expect(r.container.querySelector('[data-save-queue]')).toBeNull();
		r.unmount();
		fakeMe.feats = { queue_edit: true };
		const r2 = await setup(true);
		expect(r2.container.querySelector('[data-save-queue]')).toBeNull();
		r2.unmount();
	});

	test('saves under the suggested name, then offers to open the playlist', async () => {
		const r = await setup();
		await fireEvent.click(r.container.querySelector('[data-save-queue]')!);
		const input = document.querySelector<HTMLInputElement>('[data-save-name]')!;
		expect(input.value).toBe('Kind of Blue');
		await fireEvent.click(document.querySelector('[data-save-ok]')!);
		await waitFor(() => expect(document.querySelector('[data-save-queue-dialog]')).toBeNull());
		expect(sent).toEqual([{ player: P, name: 'Kind of Blue', replace: false }]);
		const snack = document.querySelector('[data-snackbar]')!;
		await waitFor(() => expect(snack.textContent).toContain('Saved as “Kind of Blue”'));
		const open = [...snack.querySelectorAll('button')].find((b) => b.textContent === 'Open')!;
		await fireEvent.click(open);
		expect(goto).toHaveBeenCalledWith('#/playlist/9', {});
		r.unmount();
	});

	test('an existing name is replaced only when asked', async () => {
		answer((_name, replace) => (replace ? { saved: { id: 5, name: 'Road Trip' } } : { exists: { id: 5, name: 'Road Trip' } }));
		const r = await setup();
		await fireEvent.click(r.container.querySelector('[data-save-queue]')!);
		const input = document.querySelector<HTMLInputElement>('[data-save-name]')!;
		await fireEvent.input(input, { target: { value: 'road trip' } });
		await fireEvent.click(document.querySelector('[data-save-ok]')!);
		await waitFor(() => expect(document.querySelector('[data-save-exists]')?.textContent).toContain('“Road Trip” already exists'));
		expect(sent).toEqual([{ player: P, name: 'road trip', replace: false }]);
		// Change name goes back to the name; Replace overwrites.
		await fireEvent.click(document.querySelector('[data-save-rename]')!);
		expect(document.querySelector('[data-save-name]')).not.toBeNull();
		await fireEvent.click(document.querySelector('[data-save-ok]')!);
		await waitFor(() => expect(document.querySelector('[data-save-replace]')).not.toBeNull());
		await fireEvent.click(document.querySelector('[data-save-replace]')!);
		await waitFor(() => expect(document.querySelector('[data-save-queue-dialog]')).toBeNull());
		expect(sent.at(-1)).toEqual({ player: P, name: 'road trip', replace: true });
		r.unmount();
	});

	test('a bad name is explained and never sent', async () => {
		const r = await setup();
		await fireEvent.click(r.container.querySelector('[data-save-queue]')!);
		const input = document.querySelector<HTMLInputElement>('[data-save-name]')!;
		for (const bad of ['../secret', 'a/b', '.x', '']) {
			await fireEvent.input(input, { target: { value: bad } });
			expect(document.querySelector('[data-save-problem]')?.textContent, bad).toBeTruthy();
			expect(document.querySelector<HTMLButtonElement>('[data-save-ok]')!.disabled).toBe(true);
		}
		await fireEvent.submit(input.closest('form')!);
		expect(sent).toEqual([]);
		await fireEvent.click(document.querySelector('[data-save-cancel]')!);
		expect(document.querySelector('[data-save-queue-dialog]')).toBeNull();
		r.unmount();
	});

	test('refusals and an empty queue are shown in the dialog', async () => {
		answer(() => new ApiError(403, 'changing playlists needs the admin role'));
		const r = await setup();
		await fireEvent.click(r.container.querySelector('[data-save-queue]')!);
		await fireEvent.click(document.querySelector('[data-save-ok]')!);
		await waitFor(() => expect(document.querySelector('[data-save-error]')?.textContent).toContain('needs the admin role'));
		answer(() => ({ empty: true }));
		await fireEvent.click(document.querySelector('[data-save-ok]')!);
		await waitFor(() => expect(document.querySelector('[data-save-error]')?.textContent).toContain('queue is empty'));
		expect(document.querySelector('[data-save-queue-dialog]')).not.toBeNull();
		r.unmount();
	});
});
