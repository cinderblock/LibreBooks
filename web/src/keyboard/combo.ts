// Key combinations: parse "Mod+Shift+W", match keyboard events, format for
// people. `Mod` is Ctrl on Windows/Linux and ⌘ on macOS, so one keymap entry
// serves every OS.

export type OS = 'windows' | 'mac' | 'linux';

export interface Combo {
  /** Normalised key: 'a'–'z', '0'–'9', 'f1', 'enter', 'pageup', '[', '?'… */
  key: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

/** The subset of KeyboardEvent this module reads (so tests can pass plain objects). */
export interface KeyLike {
  key: string;
  code?: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

const ALIASES: Record<string, string> = {
  esc: 'escape',
  del: 'delete',
  ins: 'insert',
  return: 'enter',
  up: 'arrowup',
  down: 'arrowdown',
  left: 'arrowleft',
  right: 'arrowright',
  pgup: 'pageup',
  pgdn: 'pagedown',
  space: ' ',
  plus: '+',
};

export function parseCombo(text: string, os: OS): Combo {
  // Split on '+' but allow a literal '+' key at the end ("Ctrl++").
  const parts = text.endsWith('++') ? [...text.slice(0, -2).split('+'), '+'] : text.split('+');
  const c: Combo = { key: '', ctrl: false, alt: false, shift: false, meta: false };
  for (const raw of parts) {
    const p = raw.trim().toLowerCase();
    if (p === 'mod') {
      if (os === 'mac') c.meta = true;
      else c.ctrl = true;
    } else if (p === 'ctrl' || p === 'control') c.ctrl = true;
    else if (p === 'alt' || p === 'option') c.alt = true;
    else if (p === 'shift') c.shift = true;
    else if (p === 'meta' || p === 'cmd' || p === 'command') c.meta = true;
    else c.key = ALIASES[p] ?? p;
  }
  if (!c.key) throw new Error(`combo "${text}" has no key`);
  return c;
}

/**
 * The key an event means, layout-aware where it can be: a typed ASCII letter or
 * digit wins (so AZERTY users' Ctrl+W is the key labelled W), and the physical
 * code is the fallback when a modifier turned the letter into something else
 * (macOS Option+W types "∑").
 */
export function eventKey(e: KeyLike): string {
  const k = e.key;
  if (k.length === 1 && /[a-z0-9]/i.test(k)) return k.toLowerCase();
  const code = e.code ?? '';
  let m = /^Key([A-Z])$/.exec(code);
  if (m) return m[1]!.toLowerCase();
  m = /^Digit(\d)$/.exec(code);
  if (m) return m[1]!;
  if (k === 'Esc') return 'escape';
  return k.length === 1 ? k : k.toLowerCase();
}

/** Single-character punctuation (`?`, `[`, `+`) is typed with or without Shift depending on layout. */
function isPunctuation(key: string): boolean {
  return key.length === 1 && !/[a-z0-9 ]/.test(key);
}

export function matches(e: KeyLike, c: Combo): boolean {
  if (eventKey(e) !== c.key) return false;
  if (e.ctrlKey !== c.ctrl || e.altKey !== c.alt || e.metaKey !== c.meta) return false;
  return isPunctuation(c.key) ? true : e.shiftKey === c.shift;
}

/** Canonical id for maps: "ctrl+alt+shift+meta+key" order. */
export function comboId(c: Combo): string {
  return [c.ctrl && 'ctrl', c.alt && 'alt', c.shift && 'shift', c.meta && 'meta', c.key].filter(Boolean).join('+');
}

const KEY_LABEL: Record<string, string> = {
  escape: 'Esc',
  enter: 'Enter',
  tab: 'Tab',
  pageup: 'Page Up',
  pagedown: 'Page Down',
  insert: 'Insert',
  delete: 'Delete',
  backspace: 'Backspace',
  arrowup: '↑',
  arrowdown: '↓',
  arrowleft: '←',
  arrowright: '→',
  home: 'Home',
  end: 'End',
  ' ': 'Space',
};

function keyLabel(key: string): string {
  if (KEY_LABEL[key]) return KEY_LABEL[key];
  if (/^f\d+$/.test(key)) return key.toUpperCase();
  return key.length === 1 ? key.toUpperCase() : key[0]!.toUpperCase() + key.slice(1);
}

/** "Ctrl+Shift+W" on Windows/Linux, "⌘⇧W" on macOS. */
export function formatCombo(text: string, os: OS): string {
  const c = parseCombo(text, os);
  if (os === 'mac') {
    return `${c.ctrl ? '⌃' : ''}${c.alt ? '⌥' : ''}${c.shift ? '⇧' : ''}${c.meta ? '⌘' : ''}${keyLabel(c.key)}`;
  }
  return [c.ctrl && 'Ctrl', c.alt && 'Alt', c.shift && 'Shift', c.meta && 'Win', keyLabel(c.key)].filter(Boolean).join('+');
}

export function hasCommandModifier(c: Combo): boolean {
  return c.ctrl || c.alt || c.meta;
}
