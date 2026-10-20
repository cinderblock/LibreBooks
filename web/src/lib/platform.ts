// Where we're running: the Tauri desktop app or a browser, and on which OS.
// The keymap depends on both (see keyboard/keymaps.ts). `?keymap=desktop|browser`
// overrides the detection for testing, remembered in localStorage.

import type { OS } from '~/keyboard/combo';
import type { Platform } from '~/keyboard/keymaps';

const KEY = 'librebooks.keymap';

/** Inside the desktop app: Tauri's globals, or the app's own injected config. */
export function isTauri(): boolean {
  if (typeof window === 'undefined') return false;
  const injected = (window as unknown as { __LIBREBOOKS__?: { mode?: string } }).__LIBREBOOKS__;
  return injected?.mode === 'desktop' || '__TAURI_INTERNALS__' in window || '__TAURI__' in window;
}

export function detectOS(): OS {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const p = (nav.userAgentData?.platform || navigator.platform || navigator.userAgent).toLowerCase();
  if (p.includes('mac') || p.includes('iphone') || p.includes('ipad')) return 'mac';
  if (p.includes('win')) return 'windows';
  return 'linux';
}

export function detectPlatform(): Platform {
  const q = new URLSearchParams(window.location.search).get('keymap');
  if (q === 'desktop' || q === 'browser') localStorage.setItem(KEY, q);
  const stored = localStorage.getItem(KEY);
  if (stored === 'desktop' || stored === 'browser') return stored;
  return isTauri() ? 'desktop' : 'browser';
}

export function setPlatformOverride(p: Platform | null) {
  if (p) localStorage.setItem(KEY, p);
  else localStorage.removeItem(KEY);
}

/** A text-entry target, where typing keys belong to the field. */
export function isEditableTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  if (t.isContentEditable) return true;
  if (t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement) return true;
  if (t instanceof HTMLInputElement) {
    return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file'].includes(t.type);
  }
  return false;
}
