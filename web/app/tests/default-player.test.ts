// Which player the app controls when it opens (SPEC D20): this device by default, with a
// per-device, per-user override; a tab's reload keeps its selection; a browser that can't
// play falls back to the last other player, and says so.
import {
	DEFAULT_SETTING,
	cleanId,
	decideStart,
	fallbackAfterFailure,
	loadDefault,
	loadTabSelection,
	saveDefault,
	saveTabSelection,
	type StartInputs,
} from '#lib/player/default-player';

const ME = '02:4c:41:00:00:01';
const LOUNGE = 'aa:bb:cc:dd:ee:01';
const STUDY = 'aa:bb:cc:dd:ee:02';

const base: StartInputs = { setting: DEFAULT_SETTING, lastUsed: '', lastRemote: '', tabSelection: '', deviceMac: ME, deviceSupported: true };
const decide = (o: Partial<StartInputs>) => decideStart({ ...base, ...o });

class MemoryStore {
	m = new Map<string, string>();
	getItem(k: string) {
		return this.m.get(k) ?? null;
	}
	setItem(k: string, v: string) {
		this.m.set(k, v);
	}
	removeItem(k: string) {
		this.m.delete(k);
	}
}
const blocked = {
	getItem: () => {
		throw new Error('blocked');
	},
	setItem: () => {
		throw new Error('blocked');
	},
	removeItem: () => {
		throw new Error('blocked');
	},
};

describe('decideStart', () => {
	test('this device by default, on every device, whatever was used last', () => {
		expect(decide({})).toEqual({ use: 'device' });
		expect(decide({ lastUsed: LOUNGE, lastRemote: LOUNGE })).toEqual({ use: 'device' });
	});

	test('a named player', () => {
		expect(decide({ setting: { kind: 'player', id: STUDY } })).toEqual({ use: 'player', id: STUDY });
		// Upper-case ids are the same player.
		expect(decide({ setting: { kind: 'player', id: STUDY.toUpperCase() } })).toEqual({ use: 'player', id: STUDY });
		// Naming this device itself is the same as "This device".
		expect(decide({ setting: { kind: 'player', id: ME } })).toEqual({ use: 'device' });
	});

	test('last used: that player, or this device if it was this device (or nothing yet)', () => {
		const last = { kind: 'last' } as const;
		expect(decide({ setting: last, lastUsed: LOUNGE })).toEqual({ use: 'player', id: LOUNGE });
		expect(decide({ setting: last, lastUsed: ME, lastRemote: LOUNGE })).toEqual({ use: 'device' });
		expect(decide({ setting: last })).toEqual({ use: 'device' });
	});

	test("a reload keeps what the tab controlled, over the setting", () => {
		expect(decide({ tabSelection: STUDY })).toEqual({ use: 'player', id: STUDY });
		expect(decide({ setting: { kind: 'player', id: STUDY }, tabSelection: ME })).toEqual({ use: 'device' });
	});

	test("a browser that can't play falls back to the last other player, with a note", () => {
		expect(decide({ deviceSupported: false, lastRemote: LOUNGE })).toEqual({ use: 'player', id: LOUNGE, note: 'unsupported' });
		expect(decide({ deviceSupported: false })).toEqual({ use: 'auto', note: 'unsupported' });
		expect(decide({ deviceSupported: false, tabSelection: ME, lastRemote: STUDY })).toEqual({ use: 'player', id: STUDY, note: 'unsupported' });
		// The last "other" player is never this device itself.
		expect(decide({ deviceSupported: false, lastRemote: ME })).toEqual({ use: 'auto', note: 'unsupported' });
	});

	test('no device id from the sign-in: no device player to wait for', () => {
		expect(decide({ deviceMac: '', lastRemote: LOUNGE })).toEqual({ use: 'player', id: LOUNGE, note: 'unsupported' });
	});

	test('malformed ids are ignored, never passed on', () => {
		expect(decide({ setting: { kind: 'player', id: 'x; rescan' } })).toEqual({ use: 'device' });
		expect(decide({ tabSelection: '../../etc', lastUsed: '<script>' , setting: { kind: 'last' } })).toEqual({ use: 'device' });
		expect(decide({ deviceMac: 'nope', deviceSupported: true, lastRemote: 'bad' })).toEqual({ use: 'auto', note: 'unsupported' });
		expect(cleanId(42)).toBe('');
	});

	test('after a failed start: the last other player, else any', () => {
		expect(fallbackAfterFailure(LOUNGE, ME)).toEqual({ use: 'player', id: LOUNGE, note: 'failed' });
		expect(fallbackAfterFailure(ME, ME)).toEqual({ use: 'auto', note: 'failed' });
		expect(fallbackAfterFailure('', ME)).toEqual({ use: 'auto', note: 'failed' });
	});
});

describe('the setting in storage', () => {
	test('per user: another account on this device has its own', () => {
		const s = new MemoryStore();
		saveDefault('alice', { kind: 'player', id: LOUNGE }, s);
		saveDefault('bob', { kind: 'last' }, s);
		expect(loadDefault('alice', s)).toEqual({ kind: 'player', id: LOUNGE });
		expect(loadDefault('bob', s)).toEqual({ kind: 'last' });
		expect(loadDefault('carol', s)).toEqual(DEFAULT_SETTING);
		saveDefault('alice', { kind: 'device' }, s);
		expect(loadDefault('alice', s)).toEqual(DEFAULT_SETTING);
	});

	test('malformed or hostile values read as the default', () => {
		const s = new MemoryStore();
		for (const v of ['{', 'null', '42', '{"kind":"player","id":"x; rescan"}', '{"kind":"evil"}', '{"kind":"player"}', '"device"']) {
			s.m.set('anywhere.defaultPlayer.alice', v);
			expect(loadDefault('alice', s), v).toEqual(DEFAULT_SETTING);
		}
		// A bad id is never stored.
		saveDefault('alice', { kind: 'player', id: 'not a mac' }, s);
		expect(s.m.get('anywhere.defaultPlayer.alice')).toBe('"device"');
	});

	test('blocked storage never throws', () => {
		expect(loadDefault('alice', blocked)).toEqual(DEFAULT_SETTING);
		expect(() => saveDefault('alice', { kind: 'last' }, blocked)).not.toThrow();
		expect(loadTabSelection(blocked)).toBe('');
		expect(() => saveTabSelection(LOUNGE, blocked)).not.toThrow();
		expect(loadDefault('alice', undefined)).toEqual(DEFAULT_SETTING);
	});

	test("a tab's selection is kept only if it is a player id", () => {
		const s = new MemoryStore();
		saveTabSelection(LOUNGE.toUpperCase(), s);
		expect(loadTabSelection(s)).toBe(LOUNGE);
		saveTabSelection('', s);
		expect(loadTabSelection(s)).toBe('');
		s.m.set('anywhere.player.tab', 'javascript:alert(1)');
		expect(loadTabSelection(s)).toBe('');
	});
});
