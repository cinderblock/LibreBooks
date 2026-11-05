// A second top-level window of the app: in the desktop app a new LibreBooks
// window (the shell turns a same-origin `window.open` into one), in a browser a
// new tab. How two companies are open side by side. (Not the in-app windows of
// `~/windows`, which live inside one page.)

import { fixtureMode } from '~/api/select';

/** The URL a new window opens for `path`, keeping fixture mode (it is per tab). */
export function newWindowUrl(path: string): string {
  const url = new URL(path, window.location.origin);
  const mode = fixtureMode();
  if (mode !== 'off') url.searchParams.set('fixtures', mode === 'stress' ? 'stress' : '1');
  return url.pathname + url.search + url.hash;
}

export function openNewWindow(path: string): void {
  window.open(newWindowUrl(path), '_blank', 'noopener');
}

/** Ctrl/Cmd+click, Shift+click or a middle click: the user asked for a new window. */
export function wantsNewWindow(e: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; button: number }): boolean {
  return e.ctrlKey || e.metaKey || e.shiftKey || e.button === 1;
}
