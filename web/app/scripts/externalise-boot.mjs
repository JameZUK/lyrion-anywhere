// Moves SvelteKit's inline start-up script into a file, so the gateway's CSP can stay
// `script-src 'self'` with no 'unsafe-inline' and no hashes. Then refuses to finish if any
// inline script or event-handler attribute remains anywhere in the built HTML.
//
//   node scripts/externalise-boot.mjs <build-dir>
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, relative, dirname } from 'node:path';

const dir = process.argv[2];
if (!dir) throw new Error('usage: externalise-boot.mjs <build-dir>');

const htmlFiles = readdirSync(dir, { recursive: true })
	.map(String)
	.filter((f) => f.endsWith('.html'))
	.map((f) => join(dir, f));

/** Inline <script> elements (no src attribute) and their bodies. */
export const INLINE_SCRIPT = /<script(?![^>]*\bsrc=)([^>]*)>([\s\S]*?)<\/script>/gi;
/** on*="…" event-handler attributes, which a strict CSP also blocks. */
export const HANDLER_ATTR = /<[a-z][^>]*\s+on[a-z]+\s*=/i;

let moved = 0;
for (const file of htmlFiles) {
	let html = readFileSync(file, 'utf8');
	html = html.replace(INLINE_SCRIPT, (_whole, attrs, body) => {
		if (/\btype=["']?(?!module|text\/javascript)/i.test(attrs)) {
			throw new Error(`${file}: unexpected inline script type: ${attrs}`);
		}
		const hash = createHash('sha256').update(body).digest('hex').slice(0, 16);
		const name = `_app/boot.${hash}.js`;
		mkdirSync(join(dir, '_app'), { recursive: true });
		writeFileSync(join(dir, name), body);
		moved++;
		// A classic (not module, not deferred) script: SvelteKit's starter uses
		// document.currentScript, which only classic scripts have. Dynamic import() works.
		const src = relative(dirname(file), join(dir, name)).split('\\').join('/');
		return `<script src="./${src}"></script>`;
	});
	writeFileSync(file, html);
}

// Verify: nothing inline may remain.
const problems = [];
for (const file of htmlFiles) {
	const html = readFileSync(file, 'utf8');
	for (const m of html.matchAll(INLINE_SCRIPT)) problems.push(`${file}: inline script ${m[0].slice(0, 60)}`);
	if (HANDLER_ATTR.test(html)) problems.push(`${file}: event-handler attribute`);
	if (/javascript:/i.test(html)) problems.push(`${file}: javascript: URL`);
}
if (problems.length) {
	console.error(problems.join('\n'));
	process.exit(1);
}
console.log(`externalise-boot: ${moved} inline script(s) moved; ${htmlFiles.length} page(s) clean`);
