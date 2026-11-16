// The `pattern` of an authenticator-code field. Kept here, not written in the markup: in a
// Svelte attribute `{6}` is an expression, so `pattern="[0-9]{6}"` reaches the browser as
// `[0-9]6` and refuses every code ("Match the requested format").

/** Six digits, nothing else (the sign-in form, which sends the field as typed). */
export const CODE_PATTERN = '[0-9]{6}';

/** Six digits, with the space some authenticator apps show in the middle (forms that strip it). */
export const CODE_PATTERN_SPACED = '[0-9]{3} ?[0-9]{3}';
