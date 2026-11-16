// In Svelte markup `{…}` inside a quoted attribute is an expression, so a regular-expression
// quantifier written there is evaluated: `pattern="[0-9]{6}"` reaches the browser as `[0-9]6`
// and the field refuses everything. Patterns live in a module (`#lib/account/code-pattern`)
// and are passed as `pattern={NAME}`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = new URL('../../src/', import.meta.url).pathname;
const LITERAL = /\bpattern\s*=\s*(?:"[^"]*"|'[^']*')/;

test('no pattern attribute is written as a literal in Svelte markup', () => {
	const found = readdirSync(ROOT, { recursive: true, withFileTypes: true })
		.filter((d) => d.isFile() && d.name.endsWith('.svelte'))
		.map((d) => join(d.parentPath, d.name))
		.filter((f) => LITERAL.test(readFileSync(f, 'utf8')));
	assert.deepEqual(found, []);
});

test('the check itself catches a literal and passes a reference', () => {
	for (const s of ['<input pattern="[0-9]{6}" />', "<input pattern='[a-z]+' />", '<input pattern = "x" />']) assert.ok(LITERAL.test(s), s);
	for (const s of ['<input pattern={CODE_PATTERN} />', 'const pattern = /x/;']) assert.ok(!LITERAL.test(s), s);
});
