// Which keys run which commands, per platform.
//
// The desktop app (Tauri) gets QuickBooks Desktop's real keys. A browser never
// delivers some of them to the page (Ctrl+W closes the tab, Ctrl+T opens one…),
// so the browser keymap drops those and adds a documented alternate, shown in
// menus and the shortcut overlay. The OS can reserve keys too (⌘Q/⌘H/⌘M on macOS).

import { comboId, parseCombo, type OS } from './combo';
import type { CommandId } from './commands';

export type Platform = 'desktop' | 'browser';

export type Keymap = Record<CommandId, string[]>;

/** QuickBooks Desktop's keys (plus a few of our own where QBD has none). */
export const QBD_KEYMAP: Keymap = {
  'palette.open': ['Mod+K'],
  'help.shortcuts': ['F1', '?'],
  'nav.home': [],
  'form.writeChecks': ['Mod+W'],
  'form.invoice': ['Mod+I'],
  'nav.customerCenter': ['Mod+J'],
  'nav.vendorCenter': [],
  'nav.useRegister': ['Mod+R'],
  'nav.chartOfAccounts': ['Mod+A'],
  'nav.find': ['Mod+F'],
  'nav.memorizedTxns': ['Mod+T'],
  'nav.reports': [],
  'theme.toggle': [],
  'window.next': ['Ctrl+Tab', 'Ctrl+F6'],
  'window.prev': ['Ctrl+Shift+Tab', 'Ctrl+Shift+F6'],
  'window.close': ['Escape', 'Ctrl+F4'],
  'record.new': ['Mod+N'],
  'record.edit': ['Mod+E'],
  'record.delete': ['Mod+D'],
  'record.history': ['Mod+H'],
  'report.quick': ['Mod+Q'],
  'txn.memorize': ['Mod+M'],
  'txn.journal': ['Mod+Y'],
  'register.goToOther': ['Mod+G'],
  'record.print': ['Mod+P'],
  'list.show': ['Mod+L'],
  'form.saveNew': ['Alt+S'],
  'form.saveClose': ['Alt+A'],
  'txn.prev': ['PageUp'],
  'txn.next': ['PageDown'],
  // macOS keyboards have no Insert key; the second combo works everywhere.
  'grid.insertLine': ['Ctrl+Insert', 'Mod+Shift+Enter'],
  'grid.deleteLine': ['Ctrl+Delete', 'Mod+Shift+Backspace'],
};

/** Keys a browser keeps for itself on every OS. */
const BROWSER_RESERVED = ['Mod+W', 'Mod+T', 'Mod+N', 'Mod+Shift+W', 'Mod+Shift+T', 'Mod+Shift+N', 'Ctrl+Tab', 'Ctrl+Shift+Tab', 'Ctrl+PageUp', 'Ctrl+PageDown'];
/** Quit, on Linux browsers and everywhere on macOS. */
const QUIT = ['Mod+Q'];
/** macOS: hide and minimise the app. */
const MAC_RESERVED = ['Mod+Q', 'Mod+H', 'Mod+M'];

/** The replacement when a QBD key is unavailable. */
const ALTERNATE: Record<string, string> = {
  'Mod+W': 'Alt+Shift+W',
  'Mod+T': 'Alt+Shift+T',
  'Mod+N': 'Alt+Shift+N',
  'Mod+Q': 'Alt+Shift+Q',
  'Mod+H': 'Alt+Shift+H',
  'Mod+M': 'Alt+Shift+M',
  'Ctrl+Tab': 'Alt+Shift+ArrowRight',
  'Ctrl+Shift+Tab': 'Alt+Shift+ArrowLeft',
};

export function reservedCombos(platform: Platform, os: OS): string[] {
  const out: string[] = [];
  if (platform === 'browser') {
    out.push(...BROWSER_RESERVED);
    if (os !== 'windows') out.push(...QUIT);
  }
  if (os === 'mac') out.push(...MAC_RESERVED);
  return [...new Set(out)];
}

export function keymapFor(platform: Platform, os: OS): Keymap {
  const reserved = new Set(reservedCombos(platform, os).map((c) => comboId(parseCombo(c, os))));
  const out = {} as Keymap;
  for (const [command, combos] of Object.entries(QBD_KEYMAP) as Array<[CommandId, string[]]>) {
    const kept: string[] = [];
    for (const combo of combos) {
      if (!reserved.has(comboId(parseCombo(combo, os)))) kept.push(combo);
      else if (ALTERNATE[combo]) kept.push(ALTERNATE[combo]);
    }
    out[command] = [...new Set(kept)];
  }
  return out;
}

/** Pairs where the platform's key differs from QBD's, for docs and the overlay. */
export function differencesFromQbd(platform: Platform, os: OS): Array<{ command: CommandId; qbd: string[]; here: string[] }> {
  const km = keymapFor(platform, os);
  return (Object.keys(QBD_KEYMAP) as CommandId[])
    .filter((c) => QBD_KEYMAP[c].join() !== km[c].join())
    .map((c) => ({ command: c, qbd: QBD_KEYMAP[c], here: km[c] }));
}
