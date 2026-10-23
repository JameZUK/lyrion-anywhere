// "Add another device" (Settings > Account): asks for the password, shows the code and a
// QR code of the pairing link with its time left, and never echoes the password.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import AddDevice from '#lib/components/AddDevice.svelte';
import { ApiError } from '#lib/api/http';
import { createPairingCode, type PairingCode } from '#lib/api/auth';
import { decodeQr } from './helpers/qr-decode.mjs';

const settle = async () => {
	for (let i = 0; i < 5; i++) await tick();
};

/** Read the rendered QR back: modules from the SVG path, then decode. */
function readQr(container: HTMLElement): string {
	const svg = container.querySelector('svg[data-qr]')!;
	const n = Number(svg.getAttribute('viewBox')!.split(' ')[2]);
	const size = n - 8;
	const dark = new Set<string>();
	for (const m of svg.querySelector('path')!.getAttribute('d')!.matchAll(/M(\d+) (\d+)h(\d+)/g)) {
		for (let k = 0; k < Number(m[3]); k++) dark.add(`${Number(m[1]) - 4 + k},${Number(m[2]) - 4}`);
	}
	return decodeQr(size, (x, y) => (dark.has(`${x},${y}`) ? 1 : 0)).text;
}

async function submit(container: HTMLElement, password = 'correct horse battery') {
	const input = container.querySelector('input[type=password]') as HTMLInputElement;
	await fireEvent.input(input, { target: { value: password } });
	await fireEvent.submit(container.querySelector('form[data-add-device]')!);
	await settle();
}

describe('Add another device', () => {
	afterEach(() => vi.useRealTimers());

	test('shows the code, a QR code of the pairing link and the time left', async () => {
		vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
		let clock = 1_000_000;
		const create = vi.fn(async (_pw: string): Promise<PairingCode> => ({ code: 'ABCDEFGHJK', expires_in: 600 }));
		const { container } = render(AddDevice, { create, server: 'https://music.example.com', now: () => clock });
		await submit(container);
		expect(create).toHaveBeenCalledWith('correct horse battery');
		expect(container.querySelector('[data-code]')!.textContent).toBe('ABCDE-FGHJK');
		expect(readQr(container)).toBe('lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK');
		expect(container.querySelector('[data-countdown]')!.textContent).toContain('Expires in 10:00');
		expect(container.textContent).not.toContain('correct horse battery');

		// The countdown runs, and expiry hides the QR code.
		clock += 65_000;
		vi.advanceTimersByTime(1000);
		await settle();
		expect(container.querySelector('[data-countdown]')!.textContent).toContain('Expires in 8:55');
		clock += 600_000;
		vi.advanceTimersByTime(1000);
		await settle();
		vi.useRealTimers();
		expect(container.querySelector('svg[data-qr]')).toBeNull();
		expect(container.querySelector('[data-countdown]')!.textContent).toContain('expired');

		await fireEvent.click(screen.getByText('Done'));
		expect(container.querySelector('[data-code]')).toBeNull();
		// The password is not kept.
		expect((container.querySelector('input[type=password]') as HTMLInputElement).value).toBe('');
	});

	test('no QR code when the page is not on a usable address', async () => {
		const create = async (): Promise<PairingCode> => ({ code: 'ABCDEFGHJK', expires_in: 600 });
		const { container } = render(AddDevice, { create, server: 'http://music.example.com' });
		await submit(container);
		expect(container.querySelector('svg[data-qr]')).toBeNull();
		expect(container.querySelector('[data-no-qr]')).not.toBeNull();
		expect(container.querySelector('[data-code]')!.textContent).toBe('ABCDE-FGHJK');
	});

	test('refusals read clearly, and a hostile reply shows nothing', async () => {
		for (const [err, want] of [
			[new ApiError(401, 'invalid credentials'), 'Password not right.'],
			[new ApiError(429, 'too many attempts; try again later'), 'Too many codes'],
		] as const) {
			const { container, unmount } = render(AddDevice, { create: () => Promise.reject(err), server: 'https://music.example.com' });
			await submit(container, 'wrong password!');
			expect(container.querySelector('[data-add-device-error]')!.textContent).toContain(want);
			expect(container.querySelector('[data-code]')).toBeNull();
			unmount();
		}
		const { container } = render(AddDevice, {
			create: async () => ({ code: '<img src=x onerror=alert(1)>', expires_in: 600 }),
			server: 'https://music.example.com',
		});
		await submit(container);
		expect(container.querySelector('[data-code]')).toBeNull();
		expect(container.querySelector('img')).toBeNull();
		expect(container.querySelector('[data-add-device-error]')).not.toBeNull();
	});

	test('the API call: bearer token, client header, password only', async () => {
		const calls: [string, RequestInit][] = [];
		vi.stubGlobal('fetch', async (url: string, init: RequestInit) => {
			calls.push([url, init]);
			return new Response(JSON.stringify({ code: 'ABCDEFGHJK', expires_in: 600 }), { status: 200 });
		});
		try {
			expect(await createPairingCode('tok', 'pw')).toEqual({ code: 'ABCDEFGHJK', expires_in: 600 });
		} finally {
			vi.unstubAllGlobals();
		}
		const [url, init] = calls[0]!;
		expect(url).toBe('/v1/devices/pair');
		expect(init.method).toBe('POST');
		const headers = init.headers as Record<string, string>;
		expect(headers.Authorization).toBe('Bearer tok');
		expect(headers['X-Anywhere-Client']).toBe('1');
		expect(JSON.parse(init.body as string)).toEqual({ password: 'pw' });
	});
});
