// The open-window list, wired to the router: the URL names the active window,
// visiting a URL opens (or brings forward) its window, and switching windows
// navigates. Closing a window with unsaved changes asks first.

import { Button, Group, Modal, Text } from '@mantine/core';
import { createContext, useCallback, useContext, useEffect, useMemo, useReducer, useRef, useState, type ReactNode } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { activeWindow, EMPTY_WINDOWS, windowsReducer, type WindowDesc, type WindowsState } from './model';
import { parseWindowPath, paths } from './paths';

export interface WindowsApi {
  state: WindowsState;
  /** Company base path: `/c/<id>`. */
  base: string;
  /** Open (or bring forward) the window for a company-relative path. */
  open(path: string): void;
  activate(key: string): void;
  /** Close; asks first when the window has unsaved changes unless `force`. */
  close(key: string, opts?: { force?: boolean }): void;
  cycle(delta: 1 | -1): void;
  setDirty(key: string, dirty: boolean): void;
  setTitle(key: string, title: string): void;
  /** The path the URL had when it matched no window (for a "not found" view). */
  notFound: string | null;
}

const WindowsContext = createContext<WindowsApi | null>(null);

export function useWindows(): WindowsApi {
  const v = useContext(WindowsContext);
  if (!v) throw new Error('useWindows outside <WindowsProvider>');
  return v;
}

/** Shorthand: a function that opens a company-relative path as a window. */
export function useOpenWindow(): (path: string) => void {
  return useWindows().open;
}

export function WindowsProvider({ base, children }: { base: string; children: ReactNode }) {
  const [state, dispatch] = useReducer(windowsReducer, EMPTY_WINDOWS);
  const [confirmKey, setConfirmKey] = useState<string | null>(null);
  const [notFound, setNotFound] = useState<string | null>(null);
  const location = useLocation();
  const navigate = useNavigate();
  const stateRef = useRef(state);
  stateRef.current = state;

  const rel = location.pathname.startsWith(base) ? location.pathname.slice(base.length) + location.search : null;

  // URL → window.
  useEffect(() => {
    if (rel === null) return;
    const desc = parseWindowPath(rel);
    if (desc) {
      setNotFound(null);
      dispatch({ type: 'open', desc });
    } else setNotFound(rel);
  }, [rel]);

  // Active window → URL (after Ctrl+Tab, or closing the active window).
  const active = activeWindow(state);
  useEffect(() => {
    if (!active) {
      // Never leave the user with nothing: closing the last window shows Home.
      if (state.windows.length === 0 && rel !== null && parseWindowPath(rel)) navigate(`${base}${paths.home()}`, { replace: true });
      return;
    }
    const target = `${base}${active.path}`;
    if (target !== location.pathname + location.search) navigate(target);
    // Only react to the active window changing, not to every location change.
  }, [active?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  const open = useCallback((path: string) => navigate(`${base}${path}`), [base, navigate]);
  const activate = useCallback((key: string) => dispatch({ type: 'activate', key }), []);
  const close = useCallback((key: string, opts: { force?: boolean } = {}) => {
    if (!opts.force && stateRef.current.dirty[key]) setConfirmKey(key);
    else dispatch({ type: 'close', key });
  }, []);
  const cycle = useCallback((delta: 1 | -1) => dispatch({ type: 'cycle', delta }), []);
  const setDirty = useCallback((key: string, dirty: boolean) => dispatch({ type: 'dirty', key, dirty }), []);
  const setTitle = useCallback((key: string, title: string) => dispatch({ type: 'title', key, title }), []);

  useShortcut('window.next', () => cycle(1));
  useShortcut('window.prev', () => cycle(-1));

  // Leaving the page with unsaved work: let the browser ask.
  const anyDirty = Object.keys(state.dirty).length > 0;
  useEffect(() => {
    if (!anyDirty) return;
    const h = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener('beforeunload', h);
    return () => window.removeEventListener('beforeunload', h);
  }, [anyDirty]);

  const api = useMemo<WindowsApi>(
    () => ({ state, base, open, activate, close, cycle, setDirty, setTitle, notFound }),
    [state, base, open, activate, close, cycle, setDirty, setTitle, notFound],
  );

  const confirmTitle = confirmKey ? (state.titles[confirmKey] ?? 'this window') : '';
  return (
    <WindowsContext.Provider value={api}>
      {children}
      <Modal opened={confirmKey !== null} onClose={() => setConfirmKey(null)} title="Unsaved changes" centered>
        <Text mb="lg">
          <b>{confirmTitle}</b> has changes that haven't been saved. Close it and discard them?
        </Text>
        <Group justify="flex-end">
          <Button variant="default" onClick={() => setConfirmKey(null)} data-autofocus>
            Keep editing
          </Button>
          <Button
            color="red"
            onClick={() => {
              if (confirmKey) dispatch({ type: 'close', key: confirmKey });
              setConfirmKey(null);
            }}
          >
            Discard changes
          </Button>
        </Group>
      </Modal>
    </WindowsContext.Provider>
  );
}

// ---- per-window context -------------------------------------------------------

export interface WindowApi {
  desc: WindowDesc;
  isActive: boolean;
  /** For shortcut `enabled` checks that must be live, not render-time. */
  isActiveNow(): boolean;
  setDirty(dirty: boolean): void;
  setTitle(title: string): void;
  close(opts?: { force?: boolean }): void;
}

const WindowContext = createContext<WindowApi | null>(null);

export function WindowProvider({ desc, isActive, children }: { desc: WindowDesc; isActive: boolean; children: ReactNode }) {
  const w = useWindows();
  const activeRef = useRef(isActive);
  activeRef.current = isActive;
  const { key } = desc;
  const value = useMemo<WindowApi>(
    () => ({
      desc,
      isActive,
      isActiveNow: () => activeRef.current,
      setDirty: (d) => w.setDirty(key, d),
      setTitle: (t) => w.setTitle(key, t),
      close: (opts) => w.close(key, opts),
    }),
    // w's callbacks are stable; desc/isActive are what change.
    [desc, isActive, key, w.setDirty, w.setTitle, w.close], // eslint-disable-line react-hooks/exhaustive-deps
  );
  useShortcut('window.close', () => w.close(key), { enabled: () => activeRef.current });
  return <WindowContext.Provider value={value}>{children}</WindowContext.Provider>;
}

export function useWindow(): WindowApi {
  const v = useContext(WindowContext);
  if (!v) throw new Error('useWindow outside a window');
  return v;
}

/** `useShortcut` limited to when this window is the active one. */
export function useWindowEnabled(): () => boolean {
  return useWindow().isActiveNow;
}
