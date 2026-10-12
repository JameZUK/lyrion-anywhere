#!/usr/bin/env node
// The LMS plugin's settings page loads plain scripts, so it gets the web app's QR encoder
// (web/app/src/lib/qr/qr.js, an ES module) as a classic script that sets
// `globalThis.AnywhereQR`. This writes that copy; `--check` only compares (the plugin's
// tests run the same comparison).
//
//   node tools/sync-qr.mjs          write the plugin's copy
//   node tools/sync-qr.mjs --check  exit 1 if it is out of date
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
export const SOURCE = `${root}web/app/src/lib/qr/qr.js`;
export const TARGET = `${root}plugin/Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/qr.js`;

/** The classic-script form of the module's source. */
export function pluginCopy(source) {
	const lines = source.split('\n');
	if (lines[0].startsWith('// @ts-nocheck')) lines.shift();
	const names = [];
	const body = lines.map((line) => {
		const m = line.match(/^export (const|function) ([A-Za-z_$][\w$]*)/);
		if (!m) {
			if (/^\s*(export|import)\b/.test(line)) throw new Error(`unexpected module syntax: ${line}`);
			return line;
		}
		names.push(m[2]);
		return line.slice('export '.length);
	});
	return [
		'// GENERATED from web/app/src/lib/qr/qr.js by tools/sync-qr.mjs: edit that file, then run',
		'// `node tools/sync-qr.mjs`. Sets globalThis.AnywhereQR for the settings page.',
		'(function () {',
		"'use strict';",
		...body,
		`globalThis.AnywhereQR = Object.freeze({ ${names.join(', ')} });`,
		'})();',
		'',
	].join('\n');
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
	const want = pluginCopy(readFileSync(SOURCE, 'utf8'));
	if (process.argv.includes('--check')) {
		let have = '';
		try {
			have = readFileSync(TARGET, 'utf8');
		} catch {
			/* missing */
		}
		if (have !== want) {
			console.error(`${TARGET} is out of date: run node tools/sync-qr.mjs`);
			process.exit(1);
		}
	} else {
		writeFileSync(TARGET, want);
	}
}
