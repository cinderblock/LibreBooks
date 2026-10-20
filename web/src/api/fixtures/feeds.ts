// FIXTURE ONLY: bank feeds in memory, mirroring lb-feeds closely enough for the
// Bank Feeds window to be exercised: a queue of downloaded transactions per
// account with suggestions from rules and history, match candidates scored as
// plans/feeds.md describes, Match / Add / Ignore / Undo, connections (file,
// SimpleFIN, Direct Connect), file detection and import (a small OFX and CSV
// reader), rules, and Amazon order matching. The server's lb-feeds is the truth.

import { blankLine, blankTxn } from '../blank';
import { ApiError } from '../errors';
import { addDays, daysBetween, type IsoDate } from '~/lib/dates';
import type { AccountRole } from '../generated/AccountRole';
import type {
  Account,
  AddOutcome,
  AddRequest,
  AmazonOrder,
  AmazonPreview,
  BatchOutcome,
  Condition,
  CsvDetection,
  CsvMapping,
  Entity,
  FeedAccountSummary,
  FeedAddPreview,
  FeedConnection,
  FeedDetection,
  FeedImportRequest,
  FeedQuery,
  FeedRule,
  FeedTxn,
  IngestReport,
  MatchCandidate,
  SimpleFinClaim,
  Statement,
  StatementAccount,
  StatementTxn,
  Suggestion,
  Txn,
} from '../types';
import type { StoredTxn } from './client';

/** What the feeds need from the books. */
export interface FeedsHost {
  today: IsoDate;
  accounts(): Account[];
  entities(): Entity[];
  roleAccount(role: AccountRole): Account | undefined;
  txns(): StoredTxn[];
  txn(id: string): StoredTxn | undefined;
  /** Post and store a transaction (throws the posting error). */
  createTxn(txn: Txn): StoredTxn;
  deleteTxn(id: string): void;
  /** The bank-feed mark on a register line (QBD's lightning bolt). */
  setDownloaded(txnId: string, accountId: string, downloaded: boolean): void;
  emit(kind: string, id: string, action: string): void;
}

const DEFAULT_MAPPING: CsvMapping = {
  delimiter: ',',
  skip_rows: 0,
  has_header: true,
  date: { Name: 'Date' },
  date_order: 'Mdy',
  amount: { Signed: { column: { Name: 'Amount' }, positive_is: 'In' } },
  payee: { Name: 'Description' },
  memo: null,
  check_number: null,
  fitid: null,
  balance: null,
};

/** Payee text the way lb-feeds normalises it before comparing: upper-case words, no card noise. */
export function cleanPayee(s: string | null | undefined): string {
  return (s ?? '')
    .toUpperCase()
    .replace(/\b(POS|DEBIT|CREDIT|PURCHASE|PAYMENT|WEB|PMT|ACH|CHECKCARD|SQ|TST)\b\*?/g, ' ')
    .replace(/[#*][\w-]*/g, ' ')
    .replace(/\d{4,}/g, ' ')
    .replace(/[^A-Z0-9 ]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Token Dice coefficient, 0–1. */
function similarity(a: string, b: string): number {
  const ta = new Set(cleanPayee(a).split(' ').filter((w) => w.length > 1));
  const tb = new Set(cleanPayee(b).split(' ').filter((w) => w.length > 1));
  if (!ta.size || !tb.size) return 0;
  let both = 0;
  for (const w of ta) if (tb.has(w)) both++;
  return (2 * both) / (ta.size + tb.size);
}

function fitidFor(date: string, amount: number, payee: string | null, n: number): string {
  let h = 0;
  for (const ch of `${date}|${amount}|${cleanPayee(payee)}`) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return `lb1:${h.toString(16).padStart(8, '0')}:${n}`;
}

export class FixtureFeeds {
  txns: FeedTxn[] = [];
  connections: FeedConnection[] = [];
  rules: FeedRule[] = [];
  private seq = 0;

  constructor(private host: FeedsHost) {
    this.seed();
  }

  // ---- seed -------------------------------------------------------------------

  private seed() {
    const acct = (name: string) => this.host.accounts().find((a) => a.full_name === name);
    const ent = (name: string) => this.host.entities().find((e) => e.full_name === name);
    const checking = acct('Checking');
    const visa = acct('Visa');
    if (!checking || !visa) return;
    const now = `${this.host.today}T08:00:00Z`;
    this.connections = [
      {
        id: '00000000-0000-4000-a000-000000000001',
        name: 'First Valley Bank (Web Connect file)',
        account_id: checking.id,
        settings: { provider: 'File', csv_mapping: null },
        account_key: 'fvb-4471',
        reverse_signs: false,
        match_days: 10,
        not_before: null,
        last_sync_at: `${addDays(this.host.today, -2)}T14:03:00Z`,
        last_error: null,
        edit_seq: 1,
      },
      {
        id: '00000000-0000-4000-a000-000000000002',
        name: 'Business Visa (SimpleFIN)',
        account_id: visa.id,
        settings: { provider: 'SimpleFin', access: 'secret:feeds.fixture.simplefin-1', remote_account_id: 'ACT-4421' },
        account_key: 'simplefin-ACT-4421',
        reverse_signs: false,
        match_days: 10,
        not_before: null,
        last_sync_at: `${addDays(this.host.today, -1)}T02:00:00Z`,
        last_error: null,
        edit_seq: 1,
      },
    ];
    const rule = (id: string, name: string, priority: number, conditions: Condition[], actions: Partial<FeedRule['actions']>): FeedRule => ({
      id,
      name,
      priority: priority,
      enabled: true,
      match_all: true,
      conditions,
      actions: { rename_to: null, entity_id: null, account_id: null, class_id: null, memo: null, auto_add: false, ...actions },
      edit_seq: 1,
    });
    this.rules = [
      rule('00000000-0000-4000-b000-000000000001', 'Pacific Power', 10, [{ kind: 'Text', field: 'Payee', op: 'Contains', value: 'PACIFIC POWER' }], { rename_to: 'Pacific Power', entity_id: ent('Pacific Power')?.id ?? null, account_id: acct('Utilities:Electric')?.id ?? null }),
      rule('00000000-0000-4000-b000-000000000002', 'Bank fees', 20, [{ kind: 'Text', field: 'Payee', op: 'Contains', value: 'FEE' }, { kind: 'Account', account_id: checking.id }], { rename_to: 'First Valley Bank', entity_id: ent('First Valley Bank')?.id ?? null, account_id: acct('Bank Service Charges')?.id ?? null, auto_add: true }),
      rule('00000000-0000-4000-b000-000000000003', 'Amazon', 30, [{ kind: 'Text', field: 'PayeeOrMemo', op: 'StartsWith', value: 'AMAZON' }], { rename_to: 'Amazon', account_id: acct('Office Supplies')?.id ?? null, memo: 'Amazon order' }),
      rule('00000000-0000-4000-b000-000000000004', 'Fuel', 40, [{ kind: 'Text', field: 'Payee', op: 'Contains', value: 'SHELL' }, { kind: 'Direction', direction: 'Out' }], { rename_to: 'Shell', entity_id: ent('Shell')?.id ?? null, account_id: acct('Automobile Expense:Fuel')?.id ?? null }),
    ];
    // Downloads that mirror real register lines (so Match finds them), plus some new ones.
    const bankStyle = (s: string | null) => (s ?? 'UNKNOWN').toUpperCase().replace(/[^A-Z0-9 ]/g, '') + ' WEB PMT';
    const push = (accountId: string, connectionId: string, posted: string, amount: number, payee: string, memo: string | null, extra: Partial<FeedTxn> = {}) => {
      const t: FeedTxn = {
        id: `00000000-0000-4000-c000-${(++this.seq).toString(16).padStart(12, '0')}`,
        connection_id: connectionId,
        account_id: accountId,
        fitid: fitidFor(posted, amount, payee, 0),
        fitid_synthesized: false,
        posted,
        amount,
        payee,
        memo,
        check_number: null,
        trntype: amount < 0 ? 'Debit' : 'Credit',
        status: 'New',
        matched_txn_id: null,
        suggestion: null,
        source: connectionId === this.connections[0]!.id ? 'file' : 'simplefin',
        imported_at: now,
        raw: { seed: true },
        ...extra,
      };
      this.txns.push(t);
      return t;
    };
    const recent = this.host
      .txns()
      .filter((s) => !s.txn.is_void && s.postings.some((p) => p.account_id === checking.id) && daysBetween(s.txn.date, this.host.today) <= 35 && s.txn.txn_type !== 'Transfer')
      .sort((a, b) => b.txn.date.localeCompare(a.txn.date))
      .slice(0, 4);
    const names = new Map(this.host.entities().map((e) => [e.id, e.full_name]));
    for (const s of recent) {
      const amount = s.postings.filter((p) => p.account_id === checking.id).reduce((t, p) => t + p.amount, 0);
      push(checking.id, this.connections[0]!.id, addDays(s.txn.date, 1) > this.host.today ? this.host.today : addDays(s.txn.date, 1), amount, bankStyle(s.txn.entity_id ? (names.get(s.txn.entity_id) ?? null) : s.txn.memo), s.txn.number ? `CHECK ${s.txn.number}` : null, { check_number: s.txn.number, trntype: s.txn.number ? 'Check' : amount < 0 ? 'Debit' : 'Deposit' });
    }
    push(checking.id, this.connections[0]!.id, addDays(this.host.today, -1), -1_200, 'MONTHLY SERVICE FEE', null, { trntype: 'Fee' });
    push(checking.id, this.connections[0]!.id, addDays(this.host.today, -3), 50_000, 'ZELLE FROM NGUYEN DAVID', 'Deposit table', { trntype: 'Deposit' });
    push(visa.id, this.connections[1]!.id, addDays(this.host.today, -2), -4_817, 'AMAZON MKTPLACE US*2K3PL9', 'AMZN.COM/BILL', { trntype: 'PointOfSale' });
    push(visa.id, this.connections[1]!.id, addDays(this.host.today, -4), -6_230, 'SHELL OIL 57444281', null, { trntype: 'PointOfSale' });
    push(visa.id, this.connections[1]!.id, addDays(this.host.today, -5), -1_450, 'SQ *RIVERSIDE CAFE', null, { trntype: 'PointOfSale' });
    push(visa.id, this.connections[1]!.id, addDays(this.host.today, -12), -9_900, 'AMAZON MKTPLACE US*7F2QQ1', 'AMZN.COM/BILL', { status: 'Ignored', trntype: 'PointOfSale' });
    for (const t of this.txns) if (t.status === 'New') t.suggestion = this.suggest(t);
  }

  // ---- rules and suggestions -------------------------------------------------------

  private ruleMatches(r: FeedRule, t: FeedTxn): boolean {
    const test = (c: Condition): boolean => {
      switch (c.kind) {
        case 'Text': {
          const field = c.field === 'Payee' ? (t.payee ?? '') : c.field === 'Memo' ? (t.memo ?? '') : `${t.payee ?? ''} ${t.memo ?? ''}`;
          const v = c.value.toLowerCase();
          const f = field.toLowerCase();
          switch (c.op) {
            case 'Contains':
              return f.includes(v);
            case 'Equals':
              return f.trim() === v.trim();
            case 'StartsWith':
              return f.trimStart().startsWith(v);
            case 'Regex':
              try {
                return new RegExp(c.value, 'i').test(field);
              } catch {
                return false;
              }
          }
          return false;
        }
        case 'AmountEquals':
          return Math.abs(t.amount) === Math.abs(c.amount);
        case 'AmountRange':
          return (c.min === null || Math.abs(t.amount) >= c.min) && (c.max === null || Math.abs(t.amount) <= c.max);
        case 'Direction':
          return c.direction === 'In' ? t.amount > 0 : t.amount < 0;
        case 'Account':
          return t.account_id === c.account_id;
      }
    };
    if (!r.enabled || !r.conditions.length) return false;
    return r.match_all ? r.conditions.every(test) : r.conditions.some(test);
  }

  /** First rule wins; else how the same payee was added last time. */
  private suggest(t: FeedTxn): Suggestion | null {
    const rule = [...this.rules].sort((a, b) => Number(a.priority) - Number(b.priority)).find((r) => this.ruleMatches(r, t));
    if (rule) {
      const a = rule.actions;
      return { rule_id: rule.id, from_history: false, payee: a.rename_to, entity_id: a.entity_id, account_id: a.account_id, class_id: a.class_id, memo: a.memo, auto_add: a.auto_add && !!a.account_id };
    }
    const key = cleanPayee(t.payee);
    const prior = key ? this.txns.find((o) => o.id !== t.id && o.status === 'Added' && o.matched_txn_id && cleanPayee(o.payee) === key) : undefined;
    const added = prior?.matched_txn_id ? this.host.txn(prior.matched_txn_id) : undefined;
    if (!added) return null;
    const line = added.txn.lines[0];
    return { rule_id: null, from_history: true, payee: null, entity_id: added.txn.entity_id, account_id: line?.account_id ?? null, class_id: line?.class_id ?? null, memo: null, auto_add: false };
  }

  // ---- queue ------------------------------------------------------------------------

  summary(): FeedAccountSummary[] {
    const out = new Map<string, FeedAccountSummary>();
    for (const c of this.connections) {
      if (!c.account_id) continue;
      const s = out.get(c.account_id) ?? { account_id: c.account_id, new: 0, matched: 0, added: 0, ignored: 0, latest_posted: null, last_imported_at: null };
      s.last_imported_at = [s.last_imported_at, c.last_sync_at].filter(Boolean).sort().at(-1) ?? null;
      out.set(c.account_id, s);
    }
    for (const t of this.txns) {
      const s = out.get(t.account_id) ?? { account_id: t.account_id, new: 0, matched: 0, added: 0, ignored: 0, latest_posted: null, last_imported_at: null };
      if (t.status === 'New') s.new++;
      else if (t.status === 'Matched') s.matched++;
      else if (t.status === 'Added') s.added++;
      else s.ignored++;
      s.latest_posted = [s.latest_posted, t.posted].filter(Boolean).sort().at(-1) ?? null;
      s.last_imported_at = [s.last_imported_at, t.imported_at].filter(Boolean).sort().at(-1) ?? null;
      out.set(t.account_id, s);
    }
    return [...out.values()];
  }

  list(q: FeedQuery = {}): FeedTxn[] {
    let rows = this.txns.filter((t) => {
      if (q.account_id && t.account_id !== q.account_id) return false;
      if (q.connection_id && t.connection_id !== q.connection_id) return false;
      if (q.statuses?.length && !q.statuses.includes(t.status)) return false;
      if (q.from && t.posted < q.from) return false;
      if (q.to && t.posted > q.to) return false;
      if (q.amount != null && Math.abs(t.amount) !== Math.abs(q.amount)) return false;
      if (q.text) {
        const hay = `${t.payee ?? ''} ${t.memo ?? ''} ${t.check_number ?? ''} ${t.fitid}`.toLowerCase();
        if (!hay.includes(q.text.toLowerCase())) return false;
      }
      return true;
    });
    rows.sort((a, b) => a.posted.localeCompare(b.posted) || a.id.localeCompare(b.id));
    if (q.descending) rows.reverse();
    const offset = q.offset ?? 0;
    rows = rows.slice(offset, q.limit != null ? offset + q.limit : undefined);
    return rows;
  }

  get(id: string): FeedTxn {
    const t = this.txns.find((x) => x.id === id);
    if (!t) throw new ApiError(404, 'not_found', 'No such downloaded transaction');
    return t;
  }

  private linkedTxnIds(): Set<string> {
    return new Set(this.txns.map((t) => t.matched_txn_id).filter((x): x is string => !!x));
  }

  candidates(id: string, days?: number): MatchCandidate[] {
    const t = this.get(id);
    const conn = this.connections.find((c) => c.id === t.connection_id);
    const n = days ?? conn?.match_days ?? 10;
    const linked = this.linkedTxnIds();
    const names = new Map(this.host.entities().map((e) => [e.id, e.full_name]));
    const out: MatchCandidate[] = [];
    for (const s of this.host.txns()) {
      if (s.txn.is_void || linked.has(s.txn.id)) continue;
      const mine = s.postings.filter((p) => p.account_id === t.account_id);
      if (!mine.length) continue;
      const amount = mine.reduce((x, p) => x + p.amount, 0);
      if (amount !== t.amount) continue;
      const d = Math.abs(daysBetween(s.txn.date, t.posted));
      if (d > n) continue;
      const reasons = ['Same amount'];
      let score = 40 + 30 * (1 - d / (n + 1));
      reasons.push(d === 0 ? 'Same day' : `${d} day${d === 1 ? '' : 's'} apart`);
      if (t.check_number && s.txn.number) {
        if (t.check_number === s.txn.number) {
          score += 30;
          reasons.push(`Check number ${s.txn.number}`);
        } else {
          score -= 40;
          reasons.push(`Different check number (${s.txn.number})`);
        }
      }
      const nameText = [s.txn.entity_id ? names.get(s.txn.entity_id) : null, ...s.txn.lines.map((l) => (l.entity_id ? names.get(l.entity_id) : null))].filter(Boolean).join(' ');
      const sim = Math.max(similarity(t.payee ?? '', nameText), similarity(t.payee ?? '', s.txn.memo ?? ''));
      if (sim > 0) {
        score += 20 * sim;
        reasons.push(`Payee looks like "${nameText || s.txn.memo}"`);
      }
      const clamped = Math.max(0, Math.min(100, Math.round(score)));
      out.push({ txn_id: s.txn.id, txn_type: s.txn.txn_type, date: s.txn.date, number: s.txn.number, entity_id: s.txn.entity_id, names: nameText || null, memo: s.txn.memo, amount, score: clamped, strong: clamped >= 65, reasons });
    }
    return out.sort((a, b) => b.score - a.score);
  }

  match(id: string, txnId: string) {
    const t = this.get(id);
    if (t.status !== 'New') throw new ApiError(400, 'invalid', 'Only a new download can be matched');
    const s = this.host.txn(txnId);
    if (!s) throw new ApiError(404, 'not_found', 'No such transaction');
    if (this.linkedTxnIds().has(txnId)) throw new ApiError(409, 'conflict', 'That transaction is already matched to another download');
    t.status = 'Matched';
    t.matched_txn_id = txnId;
    this.host.setDownloaded(txnId, t.account_id, true);
    this.host.emit('feed_txn', t.id, 'matched');
  }

  /** The transaction "Add" writes for a download, from the request and the suggestion. */
  private buildAdd(t: FeedTxn, req: AddRequest): Txn {
    const account = this.host.accounts().find((a) => a.id === t.account_id);
    if (!account) throw new ApiError(404, 'not_found', 'The download’s account is gone');
    const s = t.suggestion;
    const other = req.account_id ?? s?.account_id ?? this.host.roleAccount('AskMyAccountant')?.id ?? null;
    if (!other) throw new ApiError(400, 'invalid', 'Choose the account on the other side');
    const entity = req.entity_id ?? s?.entity_id ?? null;
    const memo = req.memo ?? s?.memo ?? t.memo ?? t.payee;
    const card = account.account_type === 'CreditCard';
    const amount = Math.abs(t.amount);
    let type: Txn['txn_type'];
    if (card) type = t.amount < 0 ? 'CreditCardCharge' : 'CreditCardCredit';
    else if (account.account_type === 'Bank' || account.account_type === 'OtherCurrentAsset') type = t.amount < 0 ? 'Check' : 'Deposit';
    else throw new ApiError(501, 'unsupported', `Adding into ${account.account_type} registers isn't supported; match it to a transaction entered by hand.`);
    const txn = blankTxn(type, t.posted);
    txn.account_id = account.id;
    txn.entity_id = type === 'Deposit' ? null : entity;
    txn.number = type === 'Check' ? (req.number ?? t.check_number) : null;
    txn.memo = memo;
    txn.amount = amount;
    txn.lines = [{ ...blankLine(type === 'Deposit' ? 'DepositLine' : 'Account'), account_id: other, amount, entity_id: type === 'Deposit' ? entity : null, class_id: req.class_id ?? s?.class_id ?? null, description: memo }];
    txn.extra = { bank_feed: { fitid: t.fitid, payee: t.payee } };
    return txn;
  }

  preview(id: string, req: AddRequest): FeedAddPreview {
    const t = this.get(id);
    const txn = this.buildAdd(t, req);
    return { txn, postings: [], total: txn.amount, sales_tax: 0 };
  }

  add(id: string, req: AddRequest): AddOutcome {
    const t = this.get(id);
    if (t.status !== 'New') throw new ApiError(400, 'invalid', 'Only a new download can be added');
    const stored = this.host.createTxn(this.buildAdd(t, req));
    t.status = 'Added';
    t.matched_txn_id = stored.txn.id;
    this.host.setDownloaded(stored.txn.id, t.account_id, true);
    this.host.emit('feed_txn', t.id, 'added');
    return { feed_txn_id: t.id, txn_id: stored.txn.id, txn_type: stored.txn.txn_type, warnings: [] };
  }

  addAll(ids: string[], req?: AddRequest | null): BatchOutcome {
    const out: BatchOutcome = { added: [], failed: [] };
    for (const id of ids) {
      try {
        out.added.push(this.add(id, req ?? { entity_id: null, account_id: null, class_id: null, memo: null, number: null }));
      } catch (e) {
        out.failed.push({ feed_txn_id: id, message: e instanceof Error ? e.message : String(e) });
      }
    }
    return out;
  }

  ignore(ids: string[]): number {
    let n = 0;
    for (const id of ids) {
      const t = this.txns.find((x) => x.id === id);
      if (t && t.status === 'New') {
        t.status = 'Ignored';
        n++;
        this.host.emit('feed_txn', t.id, 'ignored');
      }
    }
    return n;
  }

  undo(id: string): FeedTxn {
    const t = this.get(id);
    if (t.status === 'New') throw new ApiError(400, 'invalid', 'Nothing to undo');
    if (t.matched_txn_id) {
      this.host.setDownloaded(t.matched_txn_id, t.account_id, false);
      if (t.status === 'Added') this.host.deleteTxn(t.matched_txn_id);
    }
    t.status = 'New';
    t.matched_txn_id = null;
    t.suggestion = this.suggest(t);
    this.host.emit('feed_txn', t.id, 'undone');
    return t;
  }

  // ---- connections -------------------------------------------------------------

  createConnection(c: FeedConnection): FeedConnection {
    const saved: FeedConnection = { ...c, id: c.id || crypto.randomUUID(), edit_seq: 1, last_sync_at: null, last_error: null };
    this.connections.push(saved);
    this.host.emit('feed_connection', saved.id, 'created');
    return saved;
  }

  updateConnection(c: FeedConnection): FeedConnection {
    const i = this.connections.findIndex((x) => x.id === c.id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such connection');
    const cur = this.connections[i]!;
    if (Number(cur.edit_seq) !== Number(c.edit_seq)) throw new ApiError(409, 'conflict', 'Someone else changed this connection', { expected: Number(c.edit_seq), actual: Number(cur.edit_seq) });
    const saved = { ...c, edit_seq: (Number(cur.edit_seq) + 1) };
    this.connections[i] = saved;
    this.host.emit('feed_connection', saved.id, 'updated');
    return saved;
  }

  deleteConnection(id: string, editSeq: number) {
    const i = this.connections.findIndex((x) => x.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such connection');
    if (Number(this.connections[i]!.edit_seq) !== editSeq) throw new ApiError(409, 'conflict', 'Someone else changed this connection', { expected: editSeq, actual: Number(this.connections[i]!.edit_seq) });
    this.connections.splice(i, 1);
    this.host.emit('feed_connection', id, 'deleted');
  }

  setCredentials(id: string, value: string): FeedConnection {
    const c = this.connections.find((x) => x.id === id);
    if (!c) throw new ApiError(404, 'not_found', 'No such connection');
    if (!value.trim()) throw new ApiError(400, 'invalid', 'The credential is empty');
    c.last_error = null;
    c.edit_seq = (Number(c.edit_seq) + 1);
    this.host.emit('feed_connection', c.id, 'updated');
    return c;
  }

  remoteAccounts(id: string): StatementAccount[] {
    const c = this.connections.find((x) => x.id === id);
    if (!c) throw new ApiError(404, 'not_found', 'No such connection');
    if (c.settings.provider === 'File') throw new ApiError(501, 'unsupported', 'A file connection has no remote accounts');
    return [
      { bank_id: null, account_id_masked: '****4421', account_key: 'simplefin-ACT-4421', account_type: 'CreditCard', currency: 'USD', institution: 'First Valley Bank', fid: null, intu_bid: null, name: 'Business Visa', remote_id: 'ACT-4421' },
      { bank_id: '123456789', account_id_masked: '****4471', account_key: 'simplefin-ACT-4471', account_type: 'Checking', currency: 'USD', institution: 'First Valley Bank', fid: null, intu_bid: null, name: 'Everyday Checking', remote_id: 'ACT-4471' },
    ];
  }

  simplefinClaim(setupToken: string): SimpleFinClaim {
    if (!setupToken.trim() || setupToken.trim().length < 8) throw new ApiError(400, 'invalid', 'That does not look like a SimpleFIN setup token');
    return { access: 'secret:feeds.fixture.simplefin-2', accounts: this.remoteAccounts(this.connections[1]!.id), errors: [] };
  }

  /** An online sync downloads a couple of new transactions and runs the pipeline. */
  sync(id: string): IngestReport {
    const c = this.connections.find((x) => x.id === id);
    if (!c) throw new ApiError(404, 'not_found', 'No such connection');
    if (c.settings.provider === 'File') throw new ApiError(501, 'unsupported', 'A file connection is fed by importing files');
    if (!c.account_id) throw new ApiError(400, 'invalid', 'Choose the account this connection feeds first');
    const rows: StatementTxn[] = [
      { fitid: `sync-${Date.now()}-1`, fitid_synthesized: false, posted: this.host.today, amount: -2_349, payee: 'TOOL BARN #118', memo: null, check_number: null, trntype: 'PointOfSale', raw: {} },
      { fitid: `sync-${Date.now()}-2`, fitid_synthesized: false, posted: addDays(this.host.today, -1), amount: -1_199, payee: 'VALLEY INTERNET AUTOPAY', memo: null, check_number: null, trntype: 'Debit', raw: {} },
    ];
    c.last_sync_at = new Date().toISOString();
    return this.ingest(rows, c.account_id, c.id, c.settings.provider.toLowerCase());
  }

  // ---- files ---------------------------------------------------------------------

  detect(name: string, text: string): FeedDetection {
    if (/<OFX>|<STMTTRN>|OFXHEADER/i.test(text) || /\.(ofx|qfx|qbo)$/i.test(name)) return { kind: 'Ofx', statements: [this.parseOfx(text)] };
    const det = this.detectCsv(text);
    return { kind: 'Csv', csv: det };
  }

  private parseOfx(text: string): Statement {
    const tag = (block: string, t: string) => new RegExp(`<${t}>([^<\\r\\n]*)`, 'i').exec(block)?.[1]?.trim() ?? null;
    const date = (s: string | null) => (s && /^\d{8}/.test(s) ? `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}` : this.host.today);
    const cents = (s: string | null) => Math.round(Number((s ?? '0').replace(/[^0-9.-]/g, '')) * 100);
    const txns: StatementTxn[] = [...text.matchAll(/<STMTTRN>([\s\S]*?)(?=<\/STMTTRN>|<STMTTRN>|<\/BANKTRANLIST>)/gi)].map((m, i) => {
      const b = m[1]!;
      const posted = date(tag(b, 'DTPOSTED'));
      const amount = cents(tag(b, 'TRNAMT'));
      const payee = tag(b, 'NAME') ?? tag(b, 'MEMO');
      return { fitid: tag(b, 'FITID') ?? fitidFor(posted, amount, payee, i), fitid_synthesized: !tag(b, 'FITID'), posted, amount, payee, memo: tag(b, 'MEMO'), check_number: tag(b, 'CHECKNUM'), trntype: amount < 0 ? 'Debit' : 'Credit', raw: {} };
    });
    const acctId = tag(text, 'ACCTID') ?? '';
    const isCard = /<CCACCTFROM>/i.test(text);
    return {
      account: { bank_id: tag(text, 'BANKID'), account_id_masked: acctId ? `****${acctId.slice(-4)}` : '', account_key: acctId ? `ofx-${acctId.slice(-4)}` : '', account_type: isCard ? 'CreditCard' : 'Checking', currency: tag(text, 'CURDEF'), institution: tag(text, 'ORG'), fid: tag(text, 'FID'), intu_bid: tag(text, 'INTU.BID'), name: null, remote_id: null },
      ledger_balance: tag(text, 'BALAMT') ? cents(tag(text, 'BALAMT')) : null,
      available_balance: null,
      balance_date: tag(text, 'DTASOF') ? date(tag(text, 'DTASOF')) : null,
      start: tag(text, 'DTSTART') ? date(tag(text, 'DTSTART')) : null,
      end: tag(text, 'DTEND') ? date(tag(text, 'DTEND')) : null,
      transactions: txns,
      warnings: txns.length ? [] : ['No transactions were found in the file.'],
    };
  }

  private splitCsv(line: string, delimiter: string): string[] {
    const out: string[] = [];
    let cur = '';
    let q = false;
    for (let i = 0; i < line.length; i++) {
      const ch = line[i]!;
      if (ch === '"') {
        if (q && line[i + 1] === '"') {
          cur += '"';
          i++;
        } else q = !q;
      } else if (ch === delimiter && !q) {
        out.push(cur);
        cur = '';
      } else cur += ch;
    }
    out.push(cur);
    return out.map((s) => s.trim());
  }

  private detectCsv(text: string): CsvDetection {
    const lines = text.split(/\r?\n/).filter((l) => l.trim());
    const delimiter = [',', ';', '\t', '|'].sort((a, b) => (lines[0]?.split(b).length ?? 0) - (lines[0]?.split(a).length ?? 0))[0]!;
    const headers = lines.length ? this.splitCsv(lines[0]!, delimiter) : [];
    const find = (...names: string[]) => headers.find((h) => names.some((n) => h.toLowerCase().includes(n)));
    const dateCol = find('date');
    const amountCol = find('amount');
    const debitCol = find('debit', 'withdrawal');
    const creditCol = find('credit', 'deposit');
    const payeeCol = find('description', 'payee', 'name', 'merchant');
    const memoCol = find('memo', 'category', 'notes');
    const checkCol = find('check', 'chk');
    const hasHeader = !!dateCol;
    const notes: string[] = [];
    let amount: CsvMapping['amount'];
    if (amountCol) amount = { Signed: { column: { Name: amountCol }, positive_is: 'In' } };
    else if (debitCol && creditCol) amount = { DebitCredit: { debit: { Name: debitCol }, credit: { Name: creditCol } } };
    else {
      amount = { Signed: { column: { Index: 2 }, positive_is: 'In' } };
      notes.push('No amount column was recognised; guessed the third column.');
    }
    const mapping: CsvMapping = {
      ...DEFAULT_MAPPING,
      delimiter,
      has_header: hasHeader,
      date: dateCol ? { Name: dateCol } : { Index: 0 },
      amount,
      payee: payeeCol ? { Name: payeeCol } : { Index: 1 },
      memo: memoCol ? { Name: memoCol } : null,
      check_number: checkCol ? { Name: checkCol } : null,
    };
    if (!payeeCol) notes.push('No description column was recognised; guessed the second column.');
    const known = /chase|bank of america|amex|capital one|wells fargo|discover|apple card/i.test(headers.join(' '));
    const layout = /transaction date.*post date/i.test(headers.join(' ')) ? 'ChaseCard' : known ? 'Generic' : 'Generic';
    const preview = this.readCsv(text, mapping);
    return { layout, mapping, confident: !!(dateCol && (amountCol || (debitCol && creditCol)) && payeeCol && notes.length === 0), headers, notes: [...notes, ...preview.warnings], preview };
  }

  private readCsv(text: string, mapping: CsvMapping): Statement {
    const lines = text.split(/\r?\n/).filter((l) => l.trim()).slice(mapping.skip_rows);
    const headers = mapping.has_header ? this.splitCsv(lines[0]!, mapping.delimiter) : [];
    const rows = mapping.has_header ? lines.slice(1) : lines;
    const idx = (c: CsvMapping['date'] | null): number => (c === null ? -1 : 'Index' in c ? c.Index : headers.findIndex((h) => h.toLowerCase() === c.Name.toLowerCase()));
    const warnings: string[] = [];
    const parseDate = (s: string): string | null => {
      const m = /^(\d{4})-(\d{1,2})-(\d{1,2})/.exec(s);
      if (m) return `${m[1]}-${m[2]!.padStart(2, '0')}-${m[3]!.padStart(2, '0')}`;
      const p = s.split(/[/.-]/).map(Number);
      if (p.length < 3 || p.some((n) => Number.isNaN(n))) return null;
      const [a, b, c] = p as [number, number, number];
      const y = c < 100 ? 2000 + c : c;
      return mapping.date_order === 'Dmy' ? `${y}-${String(b).padStart(2, '0')}-${String(a).padStart(2, '0')}` : `${y}-${String(a).padStart(2, '0')}-${String(b).padStart(2, '0')}`;
    };
    const cents = (s: string | undefined) => {
      if (!s) return null;
      const neg = /^\(.*\)$/.test(s.trim()) || s.includes('-');
      const n = Number(s.replace(/[^0-9.]/g, ''));
      return Number.isNaN(n) ? null : Math.round(n * 100) * (neg ? -1 : 1);
    };
    const txns: StatementTxn[] = [];
    rows.forEach((line, n) => {
      const cells = this.splitCsv(line, mapping.delimiter);
      const posted = parseDate(cells[idx(mapping.date)] ?? '');
      let amount: number | null = null;
      const a = mapping.amount;
      if ('Signed' in a) {
        amount = cents(cells[idx(a.Signed.column)]);
        if (amount !== null && a.Signed.positive_is === 'Out') amount = -amount;
      } else if ('DebitCredit' in a) {
        const d = cents(cells[idx(a.DebitCredit.debit)]) ?? 0;
        const c = cents(cells[idx(a.DebitCredit.credit)]) ?? 0;
        amount = c - Math.abs(d);
      } else {
        const v = cents(cells[idx(a.AmountAndType.amount)]);
        const t = cells[idx(a.AmountAndType.type_column)] ?? '';
        amount = v === null ? null : a.AmountAndType.out_values.some((o) => o.toLowerCase() === t.toLowerCase()) ? -Math.abs(v) : Math.abs(v);
      }
      if (!posted || amount === null) {
        warnings.push(`Row ${n + 1} was skipped: no date or amount.`);
        return;
      }
      const payee = cells[idx(mapping.payee)] ?? null;
      txns.push({ fitid: cells[idx(mapping.fitid)] || fitidFor(posted, amount, payee, txns.filter((t) => t.posted === posted && t.amount === amount && cleanPayee(t.payee) === cleanPayee(payee)).length), fitid_synthesized: !cells[idx(mapping.fitid)], posted, amount, payee, memo: cells[idx(mapping.memo)] || null, check_number: cells[idx(mapping.check_number)] || null, trntype: amount < 0 ? 'Debit' : 'Credit', raw: Object.fromEntries(headers.map((h, i) => [h, cells[i] ?? ''])) });
    });
    return { account: { bank_id: null, account_id_masked: '', account_key: '', account_type: 'Checking', currency: null, institution: null, fid: null, intu_bid: null, name: null, remote_id: null }, ledger_balance: null, available_balance: null, balance_date: null, start: txns[0]?.posted ?? null, end: txns.at(-1)?.posted ?? null, transactions: txns, warnings };
  }

  import(req: FeedImportRequest, name: string, text: string): IngestReport {
    const det = this.detect(name, text);
    let statement: Statement;
    if (det.kind === 'Ofx') statement = det.statements![req.statement ?? 0] ?? det.statements![0]!;
    else {
      if (!req.mapping && !det.csv!.confident) throw new ApiError(400, 'invalid', 'The CSV layout was not recognised; check the mapping first');
      statement = this.readCsv(text, req.mapping ?? det.csv!.mapping);
    }
    let conn = req.connection_id ? this.connections.find((c) => c.id === req.connection_id) : statement.account.account_key ? this.connections.find((c) => c.account_key === statement.account.account_key) : undefined;
    const accountId = req.account_id ?? conn?.account_id;
    if (!accountId) throw new ApiError(400, 'invalid', 'Choose the account this file belongs to', { field: 'account_id' });
    if (!conn) {
      conn = this.createConnection({ id: '', name: `${statement.account.institution ?? name} (file)`, account_id: accountId, settings: { provider: 'File', csv_mapping: det.kind === 'Csv' ? (req.mapping ?? det.csv!.mapping) : null }, account_key: statement.account.account_key || null, reverse_signs: false, match_days: 10, not_before: null, last_sync_at: null, last_error: null, edit_seq: 0 });
    }
    conn.last_sync_at = new Date().toISOString();
    return this.ingest(statement.transactions, accountId, conn.id, 'file');
  }

  /** The pipeline: import (dedupe) → rules → match → auto-add. */
  private ingest(rows: StatementTxn[], accountId: string, connectionId: string, source: string): IngestReport {
    const report: IngestReport = { import: { imported: [], duplicates: 0, before_start: 0, warnings: [] }, matched: [], auto_added: [], auto_add_failures: [], waiting: 0 };
    const fresh: FeedTxn[] = [];
    const conn = this.connections.find((c) => c.id === connectionId);
    for (const r of rows) {
      if (this.txns.some((t) => t.account_id === accountId && t.fitid === r.fitid)) {
        report.import.duplicates++;
        continue;
      }
      if (conn?.not_before && r.posted < conn.not_before) {
        report.import.before_start++;
        continue;
      }
      const t: FeedTxn = { id: crypto.randomUUID(), connection_id: connectionId, account_id: accountId, fitid: r.fitid, fitid_synthesized: r.fitid_synthesized, posted: r.posted, amount: r.amount, payee: r.payee, memo: r.memo, check_number: r.check_number, trntype: r.trntype, status: 'New', matched_txn_id: null, suggestion: null, source, imported_at: new Date().toISOString(), raw: r.raw };
      this.txns.push(t);
      fresh.push(t);
      report.import.imported.push(t.id);
    }
    for (const t of fresh) t.suggestion = this.suggest(t);
    // Auto-match: exactly one strong candidate, not claimed by another new download in this batch.
    const claims = new Map<string, FeedTxn[]>();
    for (const t of fresh) {
      const strong = this.candidates(t.id).filter((c) => c.strong);
      if (strong.length === 1) claims.set(strong[0]!.txn_id, [...(claims.get(strong[0]!.txn_id) ?? []), t]);
    }
    for (const [txnId, list] of claims) {
      if (list.length !== 1) continue;
      this.match(list[0]!.id, txnId);
      report.matched.push({ feed_txn_id: list[0]!.id, txn_id: txnId });
    }
    for (const t of fresh) {
      if (t.status !== 'New') continue;
      if (t.suggestion?.auto_add && this.candidates(t.id).length === 0) {
        try {
          report.auto_added.push(this.add(t.id, { entity_id: null, account_id: null, class_id: null, memo: null, number: null }));
        } catch (e) {
          report.auto_add_failures.push({ feed_txn_id: t.id, message: e instanceof Error ? e.message : String(e) });
        }
      }
    }
    report.waiting = fresh.filter((t) => t.status === 'New').length;
    this.host.emit('feed', connectionId, 'imported');
    return report;
  }

  // ---- rules CRUD ---------------------------------------------------------------

  createRule(r: FeedRule): FeedRule {
    const saved: FeedRule = { ...r, id: r.id || crypto.randomUUID(), priority: (Math.max(0, ...this.rules.map((x) => Number(x.priority))) + 10), edit_seq: 1 };
    this.rules.push(saved);
    this.host.emit('feed_rule', saved.id, 'created');
    return saved;
  }
  updateRule(r: FeedRule): FeedRule {
    const i = this.rules.findIndex((x) => x.id === r.id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such rule');
    const cur = this.rules[i]!;
    if (Number(cur.edit_seq) !== Number(r.edit_seq)) throw new ApiError(409, 'conflict', 'Someone else changed this rule', { expected: Number(r.edit_seq), actual: Number(cur.edit_seq) });
    const saved = { ...r, priority: cur.priority, edit_seq: (Number(cur.edit_seq) + 1) };
    this.rules[i] = saved;
    this.host.emit('feed_rule', saved.id, 'updated');
    return saved;
  }
  deleteRule(id: string) {
    const i = this.rules.findIndex((x) => x.id === id);
    if (i < 0) throw new ApiError(404, 'not_found', 'No such rule');
    this.rules.splice(i, 1);
    this.host.emit('feed_rule', id, 'deleted');
  }
  reorderRules(ids: string[]): FeedRule[] {
    ids.forEach((id, i) => {
      const r = this.rules.find((x) => x.id === id);
      if (r) r.priority = ((i + 1) * 10);
    });
    this.rules.sort((a, b) => Number(a.priority) - Number(b.priority));
    this.host.emit('feed_rule', 'all', 'reordered');
    return this.rules;
  }
  applyRules(ids?: string[] | null, accountId?: string | null): number {
    let n = 0;
    for (const t of this.txns) {
      if (t.status !== 'New') continue;
      if (ids?.length && !ids.includes(t.id)) continue;
      if (accountId && t.account_id !== accountId) continue;
      t.suggestion = this.suggest(t);
      if (t.suggestion) n++;
    }
    this.host.emit('feed_txn', 'all', 'rules_applied');
    return n;
  }

  // ---- Amazon -------------------------------------------------------------------

  amazonOrders(text: string, accountId?: string | null): AmazonPreview {
    const lines = text.split(/\r?\n/).filter((l) => l.trim());
    if (!lines.length) throw new ApiError(400, 'invalid', 'The file is empty');
    const headers = this.splitCsv(lines[0]!, ',');
    const col = (...names: string[]) => headers.findIndex((h) => names.some((n) => h.toLowerCase().includes(n)));
    const cOrder = col('order id');
    const cDate = col('order date');
    const cTotal = col('total owed', 'total');
    const cName = col('product name', 'title');
    const cShip = col('ship date');
    if (cOrder < 0 || cDate < 0 || cTotal < 0) throw new ApiError(400, 'invalid', 'This is not an Amazon order history export (Order ID, Order Date and Total Owed are needed)');
    const orders = new Map<string, AmazonOrder>();
    for (const line of lines.slice(1)) {
      const c = this.splitCsv(line, ',');
      const id = c[cOrder];
      if (!id) continue;
      const total = Math.round(Number((c[cTotal] ?? '0').replace(/[^0-9.-]/g, '')) * 100);
      const o = orders.get(id) ?? { order_id: id, order_date: (c[cDate] ?? '').slice(0, 10), items: [], shipments: [], total: 0, payment: null };
      o.items.push({ name: c[cName] ?? 'Item', asin: null, quantity: 1, total, ship_date: cShip >= 0 ? c[cShip]?.slice(0, 10) || null : null });
      o.total += total;
      orders.set(id, o);
    }
    for (const o of orders.values()) {
      const byShip = new Map<string | null, number[]>();
      o.items.forEach((it, i) => byShip.set(it.ship_date, [...(byShip.get(it.ship_date) ?? []), i]));
      o.shipments = [...byShip].map(([ship_date, items]) => ({ ship_date, total: items.reduce((t, i) => t + o.items[i]!.total, 0), items }));
    }
    const used = new Set<string>();
    const matches: AmazonPreview['matches'] = [];
    for (const t of this.txns) {
      if (accountId && t.account_id !== accountId) continue;
      if (!/AMAZON|AMZN/i.test(t.payee ?? '')) continue;
      const charge = -t.amount;
      let best: { o: AmazonOrder; ship: string | null; d: number } | null = null;
      for (const o of orders.values()) {
        if (used.has(o.order_id)) continue;
        const d = daysBetween(o.order_date, t.posted);
        if (d < -2 || d > 10) continue;
        if (o.total === charge) best = !best || Math.abs(d) < Math.abs(best.d) ? { o, ship: null, d } : best;
        else for (const s of o.shipments) if (s.total === charge) best = !best || Math.abs(d) < Math.abs(best.d) ? { o, ship: s.ship_date, d } : best;
      }
      if (best) {
        used.add(best.o.order_id);
        const items = best.ship === null ? best.o.items : best.o.shipments.find((s) => s.ship_date === best!.ship)!.items.map((i) => best!.o.items[i]!);
        matches.push({ feed_txn_id: t.id, order_id: best.o.order_id, ship_date: best.ship, memo: `Amazon ${best.o.order_id}: ${items.map((i) => i.name).join('; ')}`, link: `https://www.amazon.com/gp/your-account/order-details?orderID=${best.o.order_id}` });
      }
    }
    return { matches, orders: [...orders.values()] };
  }

  amazonApply(preview: AmazonPreview): number {
    let n = 0;
    for (const m of preview.matches) {
      const t = this.txns.find((x) => x.id === m.feed_txn_id);
      if (!t) continue;
      t.memo = m.memo;
      t.raw = { ...(typeof t.raw === 'object' && t.raw ? (t.raw as object) : {}), amazon: { order_id: m.order_id, link: m.link, order: preview.orders.find((o) => o.order_id === m.order_id) } };
      n++;
      this.host.emit('feed_txn', t.id, 'updated');
    }
    return n;
  }
}
