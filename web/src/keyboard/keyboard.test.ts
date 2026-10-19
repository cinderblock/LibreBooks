import { describe, expect, test } from 'bun:test';
import { comboId, eventKey, formatCombo, matches, parseCombo, type KeyLike } from './combo';
import { COMMAND_IDS } from './commands';
import { QBD_KEYMAP, differencesFromQbd, keymapFor, reservedCombos } from './keymaps';
import { ShortcutRegistry } from './registry';

const key = (k: string, mods: Partial<KeyLike> = {}, code?: string): KeyLike => ({
  key: k,
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe('combos', () => {
  test('Mod is Ctrl off the Mac and ⌘ on it', () => {
    expect(parseCombo('Mod+W', 'windows')).toEqual({ key: 'w', ctrl: true, alt: false, shift: false, meta: false });
    expect(parseCombo('Mod+W', 'mac')).toEqual({ key: 'w', ctrl: false, alt: false, shift: false, meta: true });
  });
  test('aliases and odd keys', () => {
    expect(parseCombo('Ctrl+Ins', 'windows').key).toBe('insert');
    expect(parseCombo('Ctrl++', 'windows').key).toBe('+');
    expect(parseCombo('PageUp', 'windows').key).toBe('pageup');
  });
  test('event keys: layout letters first, physical code as fallback', () => {
    expect(eventKey(key('W', { shiftKey: true }))).toBe('w');
    expect(eventKey(key('∑', { altKey: true }, 'KeyW'))).toBe('w'); // macOS Option+W
    expect(eventKey(key('PageDown'))).toBe('pagedown');
    expect(eventKey(key('?', { shiftKey: true }, 'Slash'))).toBe('?');
  });
  test('matching', () => {
    const c = parseCombo('Ctrl+Shift+Tab', 'windows');
    expect(matches(key('Tab', { ctrlKey: true, shiftKey: true }), c)).toBe(true);
    expect(matches(key('Tab', { ctrlKey: true }), c)).toBe(false);
    // Punctuation ignores Shift (it's how '?' is typed on most layouts).
    expect(matches(key('?', { shiftKey: true }), parseCombo('?', 'windows'))).toBe(true);
    expect(matches(key('w', { ctrlKey: true, altKey: true }), parseCombo('Mod+W', 'windows'))).toBe(false);
  });
  test('formatting', () => {
    expect(formatCombo('Mod+Shift+W', 'windows')).toBe('Ctrl+Shift+W');
    expect(formatCombo('Mod+Shift+W', 'mac')).toBe('⇧⌘W');
    expect(formatCombo('Ctrl+Insert', 'linux')).toBe('Ctrl+Insert');
    expect(formatCombo('PageUp', 'windows')).toBe('Page Up');
  });
});

describe('keymaps', () => {
  test('every command has a keymap entry', () => {
    for (const id of COMMAND_IDS) expect(QBD_KEYMAP[id]).toBeDefined();
  });

  test('the desktop app gets the real QBD keys', () => {
    const km = keymapFor('desktop', 'windows');
    expect(km['form.writeChecks']).toEqual(['Mod+W']);
    expect(km['form.invoice']).toEqual(['Mod+I']);
    expect(km['nav.customerCenter']).toEqual(['Mod+J']);
    expect(km['nav.useRegister']).toEqual(['Mod+R']);
    expect(km['nav.chartOfAccounts']).toEqual(['Mod+A']);
    expect(km['nav.find']).toEqual(['Mod+F']);
    expect(km['record.new']).toEqual(['Mod+N']);
    expect(km['nav.memorizedTxns']).toEqual(['Mod+T']);
    expect(km['window.next']).toEqual(['Ctrl+Tab', 'Ctrl+F6']);
    expect(km['form.saveNew']).toEqual(['Alt+S']);
    expect(km['form.saveClose']).toEqual(['Alt+A']);
    expect(km['grid.insertLine'][0]).toBe('Ctrl+Insert');
    expect(differencesFromQbd('desktop', 'windows')).toEqual([]);
  });

  for (const os of ['windows', 'linux', 'mac'] as const) {
    test(`the ${os} browser keymap binds nothing the browser keeps`, () => {
      const km = keymapFor('browser', os);
      const reserved = new Set(reservedCombos('browser', os).map((c) => comboId(parseCombo(c, os))));
      for (const id of COMMAND_IDS) {
        for (const combo of km[id]) expect(reserved.has(comboId(parseCombo(combo, os)))).toBe(false);
      }
    });
  }

  test('browser alternates replace reserved keys and keep the rest', () => {
    const km = keymapFor('browser', 'windows');
    expect(km['form.writeChecks']).toEqual(['Alt+Shift+W']);
    expect(km['record.new']).toEqual(['Alt+Shift+N']);
    expect(km['nav.memorizedTxns']).toEqual(['Alt+Shift+T']);
    expect(km['window.next']).toEqual(['Alt+Shift+ArrowRight', 'Ctrl+F6']);
    expect(km['report.quick']).toEqual(['Mod+Q']); // Ctrl+Q is free in Windows browsers
    expect(keymapFor('browser', 'linux')['report.quick']).toEqual(['Alt+Shift+Q']); // but quits on Linux
    expect(km['form.invoice']).toEqual(['Mod+I']);
    expect(km['nav.useRegister']).toEqual(['Mod+R']);
  });

  test('macOS reserves ⌘H and ⌘M even in the desktop app', () => {
    const km = keymapFor('desktop', 'mac');
    expect(km['record.history']).toEqual(['Alt+Shift+H']);
    expect(km['txn.memorize']).toEqual(['Alt+Shift+M']);
    expect(km['form.writeChecks']).toEqual(['Mod+W']);
  });
});

describe('registry', () => {
  const make = () => new ShortcutRegistry(keymapFor('desktop', 'windows'), 'windows');
  const ctrl = (k: string) => key(k, { ctrlKey: true });

  test('dispatches to a registered handler', () => {
    const r = make();
    let hits = 0;
    r.register({ command: 'form.writeChecks', handler: () => void hits++ });
    expect(r.dispatch(ctrl('w'), { inField: false })).toBe('form.writeChecks');
    expect(hits).toBe(1);
    expect(r.dispatch(ctrl('q'), { inField: false })).toBeNull();
  });

  test('narrower scope wins; newest wins within a scope', () => {
    const r = make();
    const calls: string[] = [];
    r.register({ command: 'window.close', scope: 'window', handler: () => void calls.push('window-1') });
    r.register({ command: 'window.close', scope: 'window', handler: () => void calls.push('window-2') });
    const off = r.register({ command: 'window.close', scope: 'field', handler: () => void calls.push('field') });
    r.dispatch(key('Escape'), { inField: true });
    off();
    r.dispatch(key('Escape'), { inField: true });
    expect(calls).toEqual(['field', 'window-2']);
  });

  test('a handler can decline, and disabled handlers are skipped', () => {
    const r = make();
    const calls: string[] = [];
    r.register({ command: 'record.new', handler: () => void calls.push('outer') });
    r.register({ command: 'record.new', handler: () => false });
    r.register({ command: 'record.new', handler: () => void calls.push('hidden'), enabled: () => false });
    expect(r.dispatch(ctrl('n'), { inField: false })).toBe('record.new');
    expect(calls).toEqual(['outer']);
  });

  test('fields keep their typing keys and their Ctrl+A / Ctrl+Y', () => {
    const r = make();
    const calls: string[] = [];
    for (const command of ['help.shortcuts', 'nav.chartOfAccounts', 'txn.journal', 'form.invoice', 'txn.next'] as const) {
      r.register({ command, handler: () => void calls.push(command) });
    }
    r.dispatch(key('?', { shiftKey: true }), { inField: true }); // typed into the field
    r.dispatch(ctrl('a'), { inField: true }); // select all
    r.dispatch(ctrl('y'), { inField: true }); // redo
    r.dispatch(ctrl('i'), { inField: true }); // QBD: fires even in a field
    r.dispatch(key('PageDown'), { inField: true }); // next transaction
    r.dispatch(key('F1'), { inField: true });
    expect(calls).toEqual(['form.invoice', 'txn.next', 'help.shortcuts']);
    calls.length = 0;
    r.dispatch(key('?', { shiftKey: true }), { inField: false });
    r.dispatch(ctrl('a'), { inField: false });
    expect(calls).toEqual(['help.shortcuts', 'nav.chartOfAccounts']);
  });

  test('switching keymaps rebinds without re-registering', () => {
    const r = make();
    let hits = 0;
    r.register({ command: 'form.writeChecks', handler: () => void hits++ });
    r.setKeymap(keymapFor('browser', 'windows'));
    expect(r.dispatch(ctrl('w'), { inField: false })).toBeNull();
    expect(r.dispatch(key('W', { altKey: true, shiftKey: true }), { inField: false })).toBe('form.writeChecks');
    expect(hits).toBe(1);
  });

  test('active commands report the narrowest scope once', () => {
    const r = make();
    r.register({ command: 'window.close', scope: 'window', handler: () => {} });
    r.register({ command: 'window.close', scope: 'grid', handler: () => {} });
    r.register({ command: 'record.new', handler: () => {}, enabled: () => false });
    expect(r.activeCommands()).toEqual([{ command: 'window.close', scope: 'grid' }]);
  });
});
