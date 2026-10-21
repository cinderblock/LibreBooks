// QBD's "Open Window List" as data.
//
// Every form, register, list and report the user opens is a *window*. Windows
// stay mounted while open (switching keeps unsaved edits, scroll position and
// selection), the URL always names the active one, and Ctrl+Tab / Ctrl+F6 cycle
// through them in the order they were opened. A window's identity is its `key`:
// opening something already open just brings it forward.

export type WindowKind =
  | 'home'
  | 'accounts'
  | 'register'
  | 'customers'
  | 'vendors'
  | 'employees'
  | 'banking'
  | 'check'
  | 'txn'
  | 'reports'
  | 'report'
  | 'find'
  | 'statement-charges'
  | 'list'
  | 'record'
  | 'company'
  | 'form'
  | 'reconcile'
  | 'feeds'
  | 'memorized'
  | 'attachments'
  | 'merge'
  | 'account'
  | 'users'
  | 'enrollments'
  | 'server'
  | 'backups'
  | 'import';

export interface WindowDesc {
  key: string;
  kind: WindowKind;
  /** Route params and query values. */
  params: Record<string, string>;
  /** Path inside the company, with its query: `/accounts/abc/register`. */
  path: string;
}

export interface WindowsState {
  windows: WindowDesc[];
  active: string | null;
  /** Most recently used first: where focus goes when a window closes. */
  mru: string[];
  dirty: Record<string, boolean>;
  titles: Record<string, string>;
}

export type WindowsAction =
  | { type: 'open'; desc: WindowDesc }
  | { type: 'activate'; key: string }
  | { type: 'close'; key: string }
  | { type: 'cycle'; delta: 1 | -1 }
  | { type: 'dirty'; key: string; dirty: boolean }
  | { type: 'title'; key: string; title: string }
  | { type: 'closeAll' };

export const EMPTY_WINDOWS: WindowsState = { windows: [], active: null, mru: [], dirty: {}, titles: {} };

function touch(mru: string[], key: string): string[] {
  return [key, ...mru.filter((k) => k !== key)];
}

function omit<T>(rec: Record<string, T>, key: string): Record<string, T> {
  if (!(key in rec)) return rec;
  const { [key]: _gone, ...rest } = rec;
  return rest;
}

export function windowsReducer(state: WindowsState, action: WindowsAction): WindowsState {
  switch (action.type) {
    case 'open': {
      const i = state.windows.findIndex((w) => w.key === action.desc.key);
      const windows =
        i < 0
          ? [...state.windows, action.desc]
          : state.windows.map((w, j) => (j === i ? action.desc : w));
      return { ...state, windows, active: action.desc.key, mru: touch(state.mru, action.desc.key) };
    }
    case 'activate': {
      if (!state.windows.some((w) => w.key === action.key) || state.active === action.key) return state;
      return { ...state, active: action.key, mru: touch(state.mru, action.key) };
    }
    case 'close': {
      if (!state.windows.some((w) => w.key === action.key)) return state;
      const windows = state.windows.filter((w) => w.key !== action.key);
      const mru = state.mru.filter((k) => k !== action.key);
      const active = state.active === action.key ? (mru[0] ?? null) : state.active;
      return { windows, active, mru, dirty: omit(state.dirty, action.key), titles: omit(state.titles, action.key) };
    }
    case 'cycle': {
      const n = state.windows.length;
      if (n < 2) return state;
      const i = state.windows.findIndex((w) => w.key === state.active);
      const next = state.windows[(((i + action.delta) % n) + n) % n]!;
      return { ...state, active: next.key, mru: touch(state.mru, next.key) };
    }
    case 'dirty': {
      if (!!state.dirty[action.key] === action.dirty) return state;
      return { ...state, dirty: action.dirty ? { ...state.dirty, [action.key]: true } : omit(state.dirty, action.key) };
    }
    case 'title': {
      if (state.titles[action.key] === action.title) return state;
      return { ...state, titles: { ...state.titles, [action.key]: action.title } };
    }
    case 'closeAll':
      return EMPTY_WINDOWS;
  }
}

export function activeWindow(state: WindowsState): WindowDesc | null {
  return state.windows.find((w) => w.key === state.active) ?? null;
}
