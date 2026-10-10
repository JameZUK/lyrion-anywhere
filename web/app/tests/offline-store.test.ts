// Download storage: round trips, and each user's downloads kept apart from everyone else's.
import { MemoryBackend, OfflineStore } from '$lib/offline/store';
import { blobBytes, key, stored } from './helpers/fake-downloads';

const store = () => new OfflineStore(new MemoryBackend());

test('a track, its audio pieces and its cover round-trip', async () => {
	const s = store();
	await s.putChunk('alice', key(1), 0, new Blob([new Uint8Array([1, 2])]));
	await s.putChunk('alice', key(1), 1, new Blob([new Uint8Array([3, 4])]));
	await s.putTrack(stored({ chunks: 2 }));
	await s.putCover('alice', 7, new Blob(['img'], { type: 'image/jpeg' }));
	expect(await s.listTracks('alice')).toEqual([stored({ chunks: 2 })]);
	const audio = await s.getAudio('alice', key(1));
	expect(audio?.type).toBe('audio/mpeg');
	expect([...(await blobBytes(audio!))]).toEqual([1, 2, 3, 4]);
	expect((await s.getCover('alice', 7))?.type).toBe('image/jpeg');
});

test('pieces are read in order past ten (keys sort as numbers)', async () => {
	const s = store();
	for (let i = 11; i >= 0; i--) await s.putChunk('alice', key(1), i, new Blob([new Uint8Array([i])]));
	await s.putTrack(stored({ chunks: 12 }));
	expect([...(await blobBytes((await s.getAudio('alice', key(1)))!))]).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
});

test('an incomplete track reads as missing', async () => {
	const s = store();
	await s.putChunk('alice', key(1), 0, new Blob(['x']));
	await s.putTrack(stored({ chunks: 2 }));
	expect(await s.getAudio('alice', key(1))).toBeUndefined();
});

test('users never see each other’s downloads, even with look-alike names', async () => {
	const s = store();
	const users = ['alice', 'al', 'alice/', 'alice%2F', '../alice', 'Alice', 'al/ice'];
	for (const [i, u] of users.entries()) {
		await s.putTrack(stored({ user: u, key: key(i + 1), title: u }));
		await s.putChunk(u, key(i + 1), 0, new Blob([u]));
		await s.putCover(u, 7, new Blob([u], { type: 'image/png' }));
	}
	for (const [i, u] of users.entries()) {
		const mine = await s.listTracks(u);
		expect(mine.map((t) => t.title), u).toEqual([u]);
		expect(await s.getTrack(u, key(((i + 1) % users.length) + 1)), u).toBeUndefined();
		expect(await (await s.getCover(u, 7))!.text()).toBe(u);
	}
	await s.deleteUser('al');
	expect(await s.listTracks('al')).toEqual([]);
	for (const u of users.filter((u) => u !== 'al')) expect((await s.listTracks(u)).length, u).toBe(1);
	await s.deleteUser('alice');
	expect(await s.getCover('alice', 7)).toBeUndefined();
	expect((await s.listTracks('alice/')).length).toBe(1);
	expect(await (await s.getCover('alice/', 7))!.text()).toBe('alice/');
});

test('removing a track removes its audio, and the cover once its album is empty', async () => {
	const s = store();
	await s.putTrack(stored({ key: key(1) }));
	await s.putTrack(stored({ key: key(2) }));
	await s.putChunk('alice', key(1), 0, new Blob(['a']));
	await s.putChunk('alice', key(2), 0, new Blob(['b']));
	await s.putCover('alice', 7, new Blob(['c'], { type: 'image/png' }));
	await s.deleteTrack('alice', key(1));
	expect(await s.getCover('alice', 7)).toBeDefined();
	expect(await s.backend.list('chunks', '')).toHaveLength(1);
	await s.deleteTrack('alice', key(2));
	expect(await s.getCover('alice', 7)).toBeUndefined();
	expect(await s.backend.list('chunks', '')).toHaveLength(0);
});

test('partial downloads and the queue are kept per user', async () => {
	const s = store();
	await s.putPartial({ user: 'alice', key: key(1), format: 'mp3', total: 10, received: 4, chunks: 1, contentType: 'audio/mpeg' });
	await s.putChunk('alice', key(1), 0, new Blob(['abcd']));
	expect((await s.getPartial('alice', key(1)))?.received).toBe(4);
	expect(await s.getPartial('bob', key(1))).toBeUndefined();
	await s.discardPartial('alice', key(1));
	expect(await s.getPartial('alice', key(1))).toBeUndefined();
	expect(await s.backend.list('chunks', '')).toHaveLength(0);

	await s.saveQueue('alice', [{ id: 1, albumId: 7, choice: 'auto', paused: false }]);
	expect(await s.loadQueue('alice')).toHaveLength(1);
	expect(await s.loadQueue('bob')).toEqual([]);
	await s.saveQueue('alice', []);
	expect(await s.loadQueue('alice')).toEqual([]);
});

test('bad keys and empty users are refused', async () => {
	const s = store();
	await expect(s.getTrack('alice', '../x')).rejects.toThrow();
	await expect(s.putTrack(stored({ key: 'nothex' }))).rejects.toThrow();
	await expect(s.listTracks('')).rejects.toThrow();
	await expect(s.putCover('alice', -1, new Blob([]))).rejects.toThrow();
	await expect(s.getCover('alice', 1.5)).rejects.toThrow();
});
