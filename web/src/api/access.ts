// What each API call does to the books, and the gate that holds every change
// until the alpha-software notice is accepted (plans/alpha-notice.md).
//
// The table is exhaustive and type-checked: a new `ApiClient` method doesn't
// compile until someone decides which kind it is.

import type { ApiClient } from './client';
import type { NoticeAcceptance } from './types';

/**
 * - `read`: changes nothing (including previews, report runs and exports, and
 *   jobs that only compare).
 * - `write`: changes the books, a company, the server, or something outside
 *   (QuickBooks, a bank connection). Held until the notice is accepted.
 * - `exempt`: changes something, but must never wait on the notice: signing in
 *   and out, your own sign-in credentials and sessions, backups, and accepting
 *   the notice itself.
 */
export type Access = 'read' | 'write' | 'exempt';

export type ApiMethod = Exclude<keyof ApiClient, 'mode'>;

export const API_ACCESS: { readonly [M in ApiMethod]: Access } = {
  health: 'read',
  session: 'read',
  companies: 'read',
  createCompany: 'write',
  importCompany: 'write',

  loginStart: 'exempt',
  loginFinish: 'exempt',
  enrolStart: 'exempt',
  enrolFinish: 'exempt',
  logout: 'exempt',
  acceptAlphaNotice: 'exempt',

  companyInfo: 'read',
  list: 'read',
  getRecord: 'read',
  createRecord: 'write',
  updateRecord: 'write',
  deleteRecord: 'write',

  txns: 'read',
  txn: 'read',
  createTxn: 'write',
  updateTxn: 'write',
  previewTxn: 'read',
  voidTxn: 'write',
  deleteTxn: 'write',
  nextNumber: 'read',
  history: 'read',

  register: 'read',
  setCleared: 'write',
  balances: 'read',
  nameBalances: 'read',
  open: 'read',
  openOrders: 'read',
  fulfilment: 'read',
  undeposited: 'read',

  reportCatalog: 'read',
  runReport: 'read',
  exportReport: 'read',

  saved: 'read',
  createSaved: 'write',
  updateSaved: 'write',
  deleteSaved: 'write',
  memorizedDue: 'read',
  // A fresh, unsaved transaction to fill in; saving it is the write.
  useMemorized: 'read',

  reconcileOverview: 'read',
  startReconcile: 'write',
  reconciliation: 'read',
  markReconcile: 'write',
  finishReconcile: 'write',
  discardReconcile: 'write',
  undoReconcile: 'write',

  feedsSummary: 'read',
  feedTxns: 'read',
  feedTxn: 'read',
  // Reads the file to describe it; nothing is kept until `feedImport`.
  feedDetect: 'read',
  feedImport: 'write',
  feedCandidates: 'read',
  feedMatch: 'write',
  feedPreview: 'read',
  feedAdd: 'write',
  feedAddAll: 'write',
  feedIgnore: 'write',
  feedUndo: 'write',
  feedConnections: 'read',
  createFeedConnection: 'write',
  updateFeedConnection: 'write',
  deleteFeedConnection: 'write',
  setFeedCredentials: 'write',
  syncFeedConnection: 'write',
  feedRemoteAccounts: 'read',
  // Spends the one-time setup token.
  simplefinClaim: 'write',
  feedRules: 'read',
  createFeedRule: 'write',
  updateFeedRule: 'write',
  deleteFeedRule: 'write',
  reorderFeedRules: 'write',
  applyFeedRules: 'write',
  amazonOrders: 'read',
  amazonApply: 'write',

  users: 'read',
  user: 'read',
  updateUser: 'write',
  deleteUser: 'write',
  grantAccess: 'write',
  revokeAccess: 'write',
  enrollments: 'read',
  createEnrollment: 'write',
  // Shutting a door is never held up.
  revokeEnrollment: 'exempt',

  passkeys: 'read',
  renamePasskey: 'exempt',
  deletePasskey: 'exempt',
  devices: 'read',
  revokeDevice: 'exempt',
  sessions: 'read',
  revokeSession: 'exempt',

  server: 'read',
  putServer: 'write',
  secrets: 'read',
  putSecret: 'write',
  deleteSecret: 'write',
  update: 'read',
  checkUpdate: 'read',
  installUpdate: 'write',
  tunnel: 'read',
  putTunnel: 'write',
  serverBackupsStatus: 'read',

  putCompanyInfo: 'write',
  closingDate: 'read',
  putClosingDate: 'write',
  backups: 'read',
  backupsStatus: 'read',
  runBackup: 'exempt',
  verifyBackup: 'read',
  // Into a NEW company: still a change.
  restoreBackup: 'write',
  exportCompany: 'read',
  attachments: 'read',
  uploadAttachment: 'write',
  attachRecord: 'write',
  deleteAttachment: 'write',
  attachmentUrl: 'read',
  merge: 'write',

  qbdStatus: 'read',
  putQbdSettings: 'write',
  qbdSync: 'write',
  // Compares balances; changes nothing.
  qbdParityRun: 'read',
  qbdParity: 'read',
  qbwcFile: 'read',
  qbwcPassword: 'write',
  // Starts QuickBooks on a company file, which QuickBooks itself may change.
  openInQuickBooks: 'write',

  subscribe: 'read',
};

export interface WriteGate {
  /** Resolves when changes may go ahead; rejects to cancel the held one. */
  ensure(): Promise<void>;
  /** Hears every `session()` answer (what the server says was accepted). */
  learn?(accepted: NoticeAcceptance | null): void;
}

/**
 * `client`, with every `write` call held until `gate.ensure()` resolves (and
 * failing with its error if it rejects). Reads pass straight through. Each
 * method is bound once, so `api.txns` is the same function every time.
 */
export function gateWrites(client: ApiClient, gate: WriteGate): ApiClient {
  const bound = new Map<PropertyKey, unknown>();
  return new Proxy(client, {
    get(target, prop) {
      const value: unknown = Reflect.get(target, prop, target);
      if (typeof value !== 'function') return value;
      const cached = bound.get(prop);
      if (cached) return cached;
      const fn = value as (...args: unknown[]) => unknown;
      const access = API_ACCESS[prop as ApiMethod] as Access | undefined;
      let wrapped: unknown;
      if (access === 'write') {
        wrapped = (...args: unknown[]) => gate.ensure().then(() => fn.apply(target, args));
      } else if (prop === 'session' && gate.learn) {
        wrapped = (...args: unknown[]) =>
          (fn.apply(target, args) as ReturnType<ApiClient['session']>).then((s) => {
            gate.learn!(s.user.alpha_notice ?? null);
            return s;
          });
      } else {
        wrapped = fn.bind(target);
      }
      bound.set(prop, wrapped);
      return wrapped;
    },
  });
}
