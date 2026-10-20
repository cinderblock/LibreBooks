// React side of the shortcut registry: one window-level keydown listener, a hook
// for components to handle commands while they're mounted, and hooks to show the
// current platform's keys in menus and tooltips.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { detectOS, detectPlatform, isEditableTarget, setPlatformOverride } from '~/lib/platform';
import { formatCombo, type KeyLike, type OS } from './combo';
import type { CommandId, Scope } from './commands';
import { keymapFor, type Platform } from './keymaps';
import { ShortcutRegistry } from './registry';

interface KeyboardContextValue {
  registry: ShortcutRegistry;
  platform: Platform;
  os: OS;
  setPlatform(p: Platform): void;
}

const KeyboardContext = createContext<KeyboardContextValue | null>(null);

/** A modal (dialog, palette) owns the keyboard while it's open. */
function modalOpen(): boolean {
  return document.querySelector('[aria-modal="true"]') !== null;
}

export function KeyboardProvider({ children }: { children: ReactNode }) {
  const os = useMemo(detectOS, []);
  const [platform, setPlatformState] = useState<Platform>(detectPlatform);
  // One registry for the app's lifetime; keymap changes go through setKeymap.
  const [registry] = useState(() => new ShortcutRegistry(keymapFor(platform, os), os));

  useEffect(() => registry.setKeymap(keymapFor(platform, os)), [registry, platform, os]);

  useEffect(() => {
    // Bubble phase on window: components that handle a key themselves (a date
    // field's `+`, an open QuickFill list's Esc) call preventDefault first and
    // the registry leaves the event alone.
    const onKeyDown = (e: KeyboardEvent) => {
      // A held Esc shouldn't close window after window.
      if (e.defaultPrevented || e.isComposing || (e.repeat && e.key === 'Escape')) return;
      if (modalOpen()) return;
      const cmd = registry.dispatch(e, { inField: isEditableTarget(e.target) });
      if (cmd) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [registry]);

  const setPlatform = useCallback((p: Platform) => {
    setPlatformOverride(p);
    setPlatformState(p);
  }, []);

  const value = useMemo(() => ({ registry, platform, os, setPlatform }), [registry, platform, os, setPlatform]);
  return <KeyboardContext.Provider value={value}>{children}</KeyboardContext.Provider>;
}

export function useKeyboard(): KeyboardContextValue {
  const v = useContext(KeyboardContext);
  if (!v) throw new Error('useKeyboard outside <KeyboardProvider>');
  return v;
}

export interface ShortcutOptions {
  scope?: Scope;
  /** Static flag or a live check (e.g. "my window is the active one"). */
  enabled?: boolean | (() => boolean);
}

/**
 * Handle `command` while this component is mounted (and enabled). The handler
 * may return `false` to let an outer handler have the key.
 */
export function useShortcut(command: CommandId, handler: (e: KeyLike) => boolean | void, opts: ShortcutOptions = {}) {
  const { registry } = useKeyboard();
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  const enabledRef = useRef(opts.enabled);
  enabledRef.current = opts.enabled;
  const isStaticOff = opts.enabled === false;
  useEffect(() => {
    if (isStaticOff) return;
    return registry.register({
      command,
      scope: opts.scope,
      handler: (e) => handlerRef.current(e),
      enabled: () => {
        const en = enabledRef.current;
        return typeof en === 'function' ? en() : en !== false;
      },
    });
  }, [registry, command, opts.scope, isStaticOff]);
}

function useRegistryVersion(registry: ShortcutRegistry): number {
  const version = useRef(0);
  return useSyncExternalStore(
    useCallback(
      (cb) =>
        registry.subscribe(() => {
          version.current++;
          cb();
        }),
      [registry],
    ),
    () => version.current,
  );
}

/** Formatted keys for a command on this platform: ["Ctrl+W"] or ["Alt+Shift+W"]. */
export function useCombos(command: CommandId): string[] {
  const { registry, os } = useKeyboard();
  useRegistryVersion(registry);
  return registry.combos(command).map((c) => formatCombo(c, os));
}

/** The first key for a command, for a menu's right-hand hint; '' if none. */
export function useComboHint(command: CommandId): string {
  return useCombos(command)[0] ?? '';
}

/** Re-render on registry changes and expose a formatter (overlay, palette). */
export function useShortcutCatalog() {
  const { registry, os, platform } = useKeyboard();
  useRegistryVersion(registry);
  return {
    registry,
    os,
    platform,
    format: (command: CommandId) => registry.combos(command).map((c) => formatCombo(c, os)),
  };
}
