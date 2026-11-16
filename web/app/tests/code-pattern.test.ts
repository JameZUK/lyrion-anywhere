// The authenticator-code fields: what the browser's own check lets through. A regression
// test for `pattern="[0-9]{6}"` in Svelte markup, which compiled to `[0-9]6` and refused
// every code at sign-in.
import { expect, test } from 'vitest';
import { CODE_PATTERN, CODE_PATTERN_SPACED } from '#lib/account/code-pattern';

/** As a browser applies a `pattern` attribute: the whole value, compiled with the `v` flag. */
const field = (pattern: string) => new RegExp(`^(?:${pattern})$`, 'v');

test('a six-digit code passes every code field', () => {
	for (const p of [CODE_PATTERN, CODE_PATTERN_SPACED]) {
		for (const code of ['123456', '000000', '999996']) expect(field(p).test(code), `${p} ${code}`).toBe(true);
	}
});

test('anything else is refused', () => {
	const bad = ['', '12345', '1234567', '12345a', 'abcdef', '１２３４５６', '12 3456', '123456 ', ' 123456', '123-456', '16'];
	for (const p of [CODE_PATTERN, CODE_PATTERN_SPACED]) {
		for (const code of bad) expect(field(p).test(code), `${p} ${JSON.stringify(code)}`).toBe(false);
	}
});

test('the space an authenticator app shows is allowed only where it is stripped', () => {
	expect(field(CODE_PATTERN_SPACED).test('123 456')).toBe(true);
	expect(field(CODE_PATTERN).test('123 456')).toBe(false);
});
