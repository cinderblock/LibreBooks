// Kept apart (no imports) so Playwright's config can seed an acceptance with it.

/**
 * The alpha notice's version. Bump it when the wording changes enough that
 * everyone should read it again: anyone who accepted an older one is asked
 * once more.
 */
export const ALPHA_NOTICE_VERSION = 1;

/** Fixture mode's stand-in for the server remembering the notice (see `~/api/select.ts`). */
export const FIXTURE_ALPHA_KEY = 'librebooks.fixtures.alphaNotice';
