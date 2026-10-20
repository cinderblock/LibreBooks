// Write Checks: the first complete transaction form, and the template the
// others follow (header fields → line grids → totals → actions, client-side
// validation, server errors shown on the fields they're about, optimistic
// concurrency, QBD's keys).

import { Alert, Badge, Checkbox, Group, Loader, Stack, Tabs, Text, TextInput, Textarea } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useApi, useCompanyId, useRecordChanges } from '~/api/context';
import { isApiError, describeError } from '~/api/errors';
import { useAccountIndex, useBalanceMap, useCompanyInfo, useDeleteTxn, useEntityIndex, useItemIndex, useNextNumber, useSaveTxn, useTxn, useTxns } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { Txn } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { LineGrid, type GridColumn } from '~/inputs/LineGrid';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, ClassQuickFill, ItemQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useDateWarning } from '~/inputs/useDateWarning';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { CHECK_ACCOUNT_TYPES, naturalBalance } from '~/lib/accountTypes';
import { todayIso } from '~/lib/dates';
import { amountInWords, formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';
import { useEnterBehavior } from '~/lib/prefs';
import { useWindow, useWindows } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import {
  applyPayee,
  checkTotal,
  draftFingerprint,
  draftFromTxn,
  draftToTxn,
  hasErrors,
  isDraftEmpty,
  NO_ERRORS,
  newCheckDraft,
  serverErrors,
  setHeaderAmount,
  validateCheck,
  type CheckDraft,
  type FormErrors,
} from './checkDraft';
import {
  ConflictModal,
  flushActiveField,
  FormActionBar,
  HistoryModal,
  PostingsModal,
  RemoteChangeBanner,
  ShortcutButton,
} from './common';
import {
  blankExpense,
  blankItemRow,
  chooseItem,
  decimalTextError,
  expensesTotal,
  isBlankExpense,
  isBlankItem,
  itemsTotal,
  setItemAmount,
  setQuantity,
  setRate,
  type ExpenseRow,
  type ItemRow,
} from './purchaseLines';
import classes from './forms.module.css';

const LAST_ACCOUNT_KEY = 'librebooks.writeChecks.account';

/** `txnId` edits a saved check; without it, a new one. */
export function WriteChecks({ txnId }: { txnId?: string }) {
  const loaded = useTxn(txnId);
  if (txnId && loaded.isLoading) return <Loader m="xl" />;
  if (txnId && loaded.error) return <Alert color="red" m="md">{describeError(loaded.error)}</Alert>;
  return <CheckForm saved={loaded.data?.txn ?? null} />;
}

function CheckForm({ saved }: { saved: Txn | null }) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  const win = useWindow();
  const windows = useWindows();
  const mobile = useIsMobile();
  const [enterBehavior] = useEnterBehavior();
  const { data: info } = useCompanyInfo();
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const items = useItemIndex();
  const balances = useBalanceMap();

  const defaultAccount = () => localStorage.getItem(LAST_ACCOUNT_KEY) ?? info?.preferences.default_check_account ?? null;
  const [draft, setDraft] = useState<CheckDraft>(() => (saved ? draftFromTxn(saved) : newCheckDraft(todayIso(), defaultAccount())));
  const [baseline, setBaseline] = useState(() => draftFingerprint(draft));
  const [tab, setTab] = useState<'expenses' | 'items'>(() => (saved && saved.lines.every((l) => l.kind === 'Item') ? 'items' : 'expenses'));
  const [errors, setErrors] = useState<FormErrors>(NO_ERRORS);
  const [conflict, setConflict] = useState<Txn | null>(null);
  const [remoteChanged, setRemoteChanged] = useState(false);
  const [journalFor, setJournalFor] = useState<Txn | null>(null);
  const [historyFor, setHistoryFor] = useState<string | null>(null);
  const payeeRef = useRef<HTMLInputElement>(null);
  const latest = useRef(draft);
  latest.current = draft;

  // The company's default account arrives after first render on a cold load.
  useEffect(() => {
    if (!draft.account_id && info?.preferences.default_check_account && draft.edit_seq === 0n) {
      setDraft((d) => ({ ...d, account_id: defaultAccount() }));
    }
  }, [info]); // eslint-disable-line react-hooks/exhaustive-deps

  // A new check is dirty once anything is entered; a saved one once it differs.
  const dirty = saved ? draftFingerprint(draft) !== baseline : !isDraftEmpty(draft);
  useEffect(() => win.setDirty(dirty), [dirty]); // eslint-disable-line react-hooks/exhaustive-deps

  const bank = draft.account_id ? accounts.get(draft.account_id) : undefined;
  useEffect(() => {
    const n = saved ? `Check ${saved.number ?? ''}`.trim() : 'Write Checks';
    win.setTitle(bank ? `${n} - ${bank.name}` : n);
  }, [bank?.name, saved?.number]); // eslint-disable-line react-hooks/exhaustive-deps

  // Next check number for this account (new checks, until the user types one).
  const next = useNextNumber('Check', draft.account_id, draft.edit_seq === 0n && draft.numberIsAuto);
  useEffect(() => {
    if (next.data !== undefined && draft.numberIsAuto && draft.edit_seq === 0n) {
      setDraft((d) => (d.numberIsAuto ? { ...d, number: next.data ?? '' } : d));
    }
  }, [next.data]); // eslint-disable-line react-hooks/exhaustive-deps

  // Someone else saved this check: reload quietly if we have no edits, else say so.
  useRecordChanges('txn', saved?.id, () => {
    if (!dirty) void qc.invalidateQueries({ queryKey: qk.txn(co, saved!.id) });
    else setRemoteChanged(true);
  });
  useEffect(() => {
    if (saved && !dirty && saved.edit_seq !== draft.edit_seq) {
      const d = draftFromTxn(saved);
      setDraft(d);
      setBaseline(draftFingerprint(d));
    }
  }, [saved]); // eslint-disable-line react-hooks/exhaustive-deps

  const save = useSaveTxn();
  const del = useDeleteTxn();
  const dateWarning = useDateWarning(draft.date);

  const update = (patch: Partial<CheckDraft> | ((d: CheckDraft) => CheckDraft)) => {
    setDraft((d) => (typeof patch === 'function' ? patch(d) : { ...d, ...patch }));
    if (hasErrors(errors)) setErrors(NO_ERRORS);
  };

  const reset = (to: CheckDraft) => {
    setDraft(to);
    setBaseline(draftFingerprint(to));
    setErrors(NO_ERRORS);
    setRemoteChanged(false);
    setTab('expenses');
  };

  const doSave = async (mode: 'new' | 'close', opts: { overwrite?: boolean } = {}) => {
    await flushActiveField();
    let d = latest.current;
    const v = validateCheck(d);
    if (hasErrors(v)) {
      setErrors(v);
      if (Object.keys(v.items).length && !Object.keys(v.expenses).length) setTab('items');
      return;
    }
    if (opts.overwrite && d.base) {
      // Take the server's current edit_seq and save ours over it.
      const current = await api.txn(co, d.id);
      d = { ...d, edit_seq: current.txn.edit_seq, base: { ...d.base, edit_seq: current.txn.edit_seq } };
    }
    const txn = draftToTxn(d);
    try {
      const out = await save.mutateAsync(txn);
      if (d.account_id) localStorage.setItem(LAST_ACCOUNT_KEY, d.account_id);
      notifications.show({ message: `Check ${out.txn.number ?? ''} for ${formatMoney(out.total, { symbol: true })} saved.`, color: 'teal' });
      if (mode === 'close') {
        win.close({ force: true });
      } else if (saved) {
        // Editing an existing check: Save & New opens a fresh Write Checks.
        win.close({ force: true });
        windows.open(paths.newCheck());
      } else {
        reset(newCheckDraft(d.date, d.account_id));
        requestAnimationFrame(() => payeeRef.current?.focus());
      }
    } catch (e) {
      if (isApiError(e) && e.isStaleConflict) setConflict(txn);
      else if (isApiError(e)) setErrors(serverErrors(d, txn, e));
      else setErrors({ ...NO_ERRORS, form: [describeError(e)] });
    }
  };

  const enabled = win.isActiveNow;
  useShortcut('form.saveNew', () => void doSave('new'), { enabled });
  useShortcut('form.saveClose', () => void doSave('close'), { enabled });
  useShortcut('record.new', () => windows.open(paths.newCheck()), { enabled });
  useShortcut('txn.journal', () => setJournalFor(draftToTxn(latest.current)), { enabled });
  useShortcut('record.history', () => (saved ? setHistoryFor(saved.id) : false), { enabled });
  useShortcut('record.print', () => window.print(), { enabled });
  useShortcut('record.delete', () => (saved ? void onDelete() : false), { enabled });

  // Previous / Next check (Page Up / Page Down), in date order within the account.
  const siblings = useTxns({ types: ['Check'], account_id: draft.account_id ?? undefined }, !!draft.account_id);
  const go = (delta: 1 | -1) => {
    const list = siblings.data ?? [];
    if (!list.length) return;
    const i = saved ? list.findIndex((s) => s.id === saved.id) : list.length;
    const target = list[i + delta];
    if (!target) {
      notifications.show({ message: delta < 0 ? 'This is the first check.' : 'This is the last check.', color: 'gray' });
      return;
    }
    windows.open(paths.txn(target.id));
  };
  useShortcut('txn.prev', () => go(-1), { enabled });
  useShortcut('txn.next', () => go(1), { enabled });

  const onDelete = async () => {
    if (!saved) return;
    if (!window.confirm(`Delete check ${saved.number ?? ''}? This can't be undone.`)) return;
    try {
      await del.mutateAsync({ id: saved.id, editSeq: saved.edit_seq });
      notifications.show({ message: 'Check deleted.', color: 'gray' });
      win.close({ force: true });
    } catch (e) {
      setErrors({ ...NO_ERRORS, form: [describeError(e)] });
    }
  };

  const onPayee = async (id: string | null) => {
    const payee = id ? entities.get(id) : undefined;
    let last: Txn | null = null;
    if (payee && draft.edit_seq === 0n && checkTotal(draft) === 0) {
      try {
        const [summary] = await api.txns(co, { types: ['Check'], entity_id: payee.id, descending: true, limit: 1 });
        if (summary) last = (await api.txn(co, summary.id)).txn;
      } catch {
        // Recall is a convenience; without it the default account still applies.
      }
    }
    update((d) => applyPayee(d, payee, last));
  };

  const total = checkTotal(draft);
  const bankBalance = bank ? naturalBalance(bank.account_type, balances.get(bank.id) ?? 0) : null;
  const payee = draft.entity_id ? entities.get(draft.entity_id) : undefined;
  const address = payee ? [payee.print_as ?? payee.company_name ?? payee.name, ...(payee.bill_address.lines ?? []), [payee.bill_address.city, payee.bill_address.state, payee.bill_address.postal_code].filter(Boolean).join(' ')].filter(Boolean).join('\n') : '';
  const useClasses = info?.preferences.use_class_tracking ?? false;

  const expenseColumns = useMemo<GridColumn<ExpenseRow>[]>(() => {
    const cols: GridColumn<ExpenseRow>[] = [
      {
        key: 'account',
        header: 'Account',
        width: '30%',
        render: (c) => (
          <AccountQuickFill
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            value={c.row.account_id}
            onChange={(id) => c.update((r) => ({ ...r, account_id: id }))}
            onCommit={c.onCommit}
            error={c.error}
            inputProps={c.cellProps}
          />
        ),
      },
      {
        key: 'amount',
        header: 'Amount',
        width: 120,
        align: 'right',
        mobile: 'half',
        render: (c) => (
          <MoneyField
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            blankZero
            value={c.row.amount}
            onChange={(v) => c.update((r) => ({ ...r, amount: v }))}
            error={c.error}
            inputProps={c.cellProps}
          />
        ),
        footer: <span className={classes.total}>{formatMoney(expensesTotal(draft.expenses))}</span>,
      },
      {
        key: 'memo',
        header: 'Memo',
        mobile: 'full',
        render: (c) => (
          <TextInput
            size="xs"
            variant={c.variant === 'cell' ? 'unstyled' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            classNames={c.variant === 'cell' ? { input: 'lb-cell-text' } : undefined}
            value={c.row.memo}
            onChange={(e) => {
              const memo = e.currentTarget.value;
              c.update((r) => ({ ...r, memo }));
            }}
            onFocus={(e) => e.currentTarget.select()}
            {...c.cellProps}
          />
        ),
      },
      {
        key: 'customer',
        header: 'Customer:Job',
        width: '20%',
        mobile: 'full',
        render: (c) => (
          <NameQuickFill
            kinds={['Customer']}
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            value={c.row.entity_id}
            onChange={(id) => c.update((r) => ({ ...r, entity_id: id, billable: id ? (info?.preferences.billable_by_default ?? true) : false }))}
            onCommit={c.onCommit}
            error={c.error}
            inputProps={c.cellProps}
          />
        ),
      },
      {
        key: 'billable',
        header: 'Billable?',
        width: 70,
        align: 'center',
        mobile: 'half',
        render: (c) => (
          <Checkbox
            size="xs"
            label={c.variant === 'default' ? 'Billable' : undefined}
            styles={{ root: { display: 'flex', justifyContent: 'center' } }}
            checked={c.row.billable}
            disabled={!c.row.entity_id}
            onChange={(e) => {
              const billable = e.currentTarget.checked;
              c.update((r) => ({ ...r, billable }));
            }}
            {...c.cellProps}
          />
        ),
      },
    ];
    if (useClasses) {
      cols.push({
        key: 'class',
        header: 'Class',
        width: '14%',
        mobile: 'half',
        render: (c) => (
          <ClassQuickFill
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            value={c.row.class_id}
            onChange={(id) => c.update((r) => ({ ...r, class_id: id }))}
            onCommit={c.onCommit}
            inputProps={c.cellProps}
          />
        ),
      });
    }
    return cols;
  }, [draft.expenses, useClasses, info]); // eslint-disable-line react-hooks/exhaustive-deps

  const itemColumns = useMemo<GridColumn<ItemRow>[]>(() => {
    const cols: GridColumn<ItemRow>[] = [
      {
        key: 'item',
        header: 'Item',
        width: '20%',
        render: (c) => (
          <ItemQuickFill
            side="buy"
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            value={c.row.item_id}
            onChange={(id) => c.update((r) => chooseItem(r, id ? items.get(id) : undefined))}
            onCommit={c.onCommit}
            error={c.error}
            inputProps={c.cellProps}
          />
        ),
      },
      {
        key: 'description',
        header: 'Description',
        mobile: 'full',
        render: (c) => (
          <TextInput
            size="xs"
            variant={c.variant === 'cell' ? 'unstyled' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            classNames={c.variant === 'cell' ? { input: 'lb-cell-text' } : undefined}
            value={c.row.description}
            onChange={(e) => {
              const description = e.currentTarget.value;
              c.update((r) => ({ ...r, description }));
            }}
            onFocus={(e) => e.currentTarget.select()}
            {...c.cellProps}
          />
        ),
      },
      {
        key: 'quantity',
        header: 'Qty',
        width: 70,
        align: 'right',
        mobile: 'half',
        render: (c) => (
          <TextInput
            size="xs"
            variant={c.variant === 'cell' ? 'unstyled' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            classNames={c.variant === 'cell' ? { input: 'lb-cell-text lb-num' } : { input: 'lb-num' }}
            inputMode="decimal"
            value={c.row.quantity}
            error={c.variant === 'default' ? decimalTextError(c.row.quantity) : undefined}
            aria-invalid={!!decimalTextError(c.row.quantity)}
            onChange={(e) => {
              const q = e.currentTarget.value;
              c.update((r) => setQuantity(r, q));
            }}
            onFocus={(e) => e.currentTarget.select()}
            {...c.cellProps}
          />
        ),
      },
      {
        key: 'rate',
        header: 'Cost',
        width: 100,
        align: 'right',
        mobile: 'half',
        render: (c) => (
          <TextInput
            size="xs"
            variant={c.variant === 'cell' ? 'unstyled' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            classNames={c.variant === 'cell' ? { input: 'lb-cell-text lb-num' } : { input: 'lb-num' }}
            inputMode="decimal"
            value={c.row.rate}
            aria-invalid={!!decimalTextError(c.row.rate)}
            onChange={(e) => {
              const rate = e.currentTarget.value;
              c.update((r) => setRate(r, rate));
            }}
            onFocus={(e) => e.currentTarget.select()}
            {...c.cellProps}
          />
        ),
      },
      {
        key: 'amount',
        header: 'Amount',
        width: 110,
        align: 'right',
        mobile: 'half',
        render: (c) => (
          <MoneyField
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            blankZero
            value={c.row.amount}
            onChange={(v) => c.update((r) => setItemAmount(r, v))}
            error={c.error}
            inputProps={c.cellProps}
          />
        ),
        footer: <span className={classes.total}>{formatMoney(itemsTotal(draft.items))}</span>,
      },
      {
        key: 'customer',
        header: 'Customer:Job',
        width: '18%',
        mobile: 'full',
        render: (c) => (
          <NameQuickFill
            kinds={['Customer']}
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            value={c.row.entity_id}
            onChange={(id) => c.update((r) => ({ ...r, entity_id: id, billable: id ? (info?.preferences.billable_by_default ?? true) : false }))}
            onCommit={c.onCommit}
            inputProps={c.cellProps}
          />
        ),
      },
      {
        key: 'billable',
        header: 'Billable?',
        width: 70,
        align: 'center',
        mobile: 'half',
        render: (c) => (
          <Checkbox
            size="xs"
            label={c.variant === 'default' ? 'Billable' : undefined}
            styles={{ root: { display: 'flex', justifyContent: 'center' } }}
            checked={c.row.billable}
            disabled={!c.row.entity_id}
            onChange={(e) => {
              const billable = e.currentTarget.checked;
              c.update((r) => ({ ...r, billable }));
            }}
            {...c.cellProps}
          />
        ),
      },
    ];
    return cols;
  }, [draft.items, items, info]); // eslint-disable-line react-hooks/exhaustive-deps

  const onEnterSave = useCallback(() => void doSave('close'), []); // eslint-disable-line react-hooks/exhaustive-deps

  const header = (
    <>
      <Group align="flex-end" gap="md" wrap={mobile ? 'wrap' : 'nowrap'}>
        <AccountQuickFill
          label="Bank Account"
          types={CHECK_ACCOUNT_TYPES}
          value={draft.account_id}
          onChange={(id) => update((d) => ({ ...d, account_id: id, numberIsAuto: d.edit_seq === 0n ? true : d.numberIsAuto }))}
          error={errors.header.account}
          style={{ flex: mobile ? '1 1 100%' : '0 1 320px' }}
          aria-label="Bank account"
        />
        <Text size="sm" c="dimmed" pb={6}>
          Ending Balance{' '}
          <Text span fw={600} c="var(--mantine-color-text)">
            {bankBalance === null ? '—' : formatMoney(bankBalance)}
          </Text>
        </Text>
        {saved?.is_void && <Badge color="red">VOID</Badge>}
      </Group>

      <div className={mobile ? undefined : classes.checkFace}>
        <Stack gap="sm" className={mobile ? undefined : classes.wide} style={mobile ? undefined : { gridColumn: '1 / 2' }}>
          <NameQuickFill
            label="Pay to the Order of"
            value={draft.entity_id}
            onChange={(id) => void onPayee(id)}
            error={errors.header.entity}
            inputRef={payeeRef}
            autoFocus={!saved && !mobile}
            aria-label="Payee"
          />
        </Stack>
        <TextInput
          label="No."
          value={draft.to_be_printed ? 'To Print' : draft.number}
          disabled={draft.to_be_printed}
          onChange={(e) => {
            const number = e.currentTarget.value;
            update((d) => ({ ...d, number, numberIsAuto: false }));
          }}
          error={errors.header.number}
          inputMode="numeric"
          aria-label="Check number"
        />
        <DateField label="Date" value={draft.date} onChange={(date) => date && update({ date })} error={errors.header.date} warning={dateWarning} aria-label="Date" />
        <MoneyField
          label="$ Amount"
          value={total}
          onChange={(v) => update((d) => setHeaderAmount(d, v))}
          error={errors.header.amount}
          aria-label="Check amount"
          size={mobile ? 'md' : 'sm'}
        />
        {!mobile && (
          <div className={classes.full}>
            <div className={classes.words} aria-label="Amount in words">
              {total > 0 ? `${amountInWords(total)} ${'*'.repeat(Math.max(0, 60 - amountInWords(total).length))}` : ''}
            </div>
          </div>
        )}
        {!mobile && (
          <Textarea label="Address" value={address} readOnly autosize minRows={3} maxRows={5} tabIndex={-1} className={classes.wide} />
        )}
        <TextInput
          label="Memo"
          value={draft.memo}
          onChange={(e) => {
            const memo = e.currentTarget.value;
            update({ memo });
          }}
          className={mobile ? undefined : classes.wide}
          aria-label="Memo"
        />
        <Checkbox
          label="Print later"
          checked={draft.to_be_printed}
          onChange={(e) => {
            const to_be_printed = e.currentTarget.checked;
            update({ to_be_printed });
          }}
          mb={6}
        />
      </div>
    </>
  );

  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid="write-checks">
      {remoteChanged && (
        <RemoteChangeBanner
          onReload={() => {
            setRemoteChanged(false);
            setBaseline(draftFingerprint(draft)); // treat as clean so the reload effect applies
            void qc.invalidateQueries({ queryKey: qk.txn(co, saved!.id) });
          }}
        />
      )}
      {header}
      {errors.form.length > 0 && (
        <Alert color="red" role="alert">
          {errors.form.map((m) => (
            <div key={m}>{m}</div>
          ))}
        </Alert>
      )}
      <Tabs value={tab} onChange={(v) => setTab(v === 'items' ? 'items' : 'expenses')} keepMounted>
        <Tabs.List>
          <Tabs.Tab value="expenses" color={Object.keys(errors.expenses).length ? 'red' : undefined}>
            Expenses <Text span fw={600}>{formatMoney(expensesTotal(draft.expenses))}</Text>
          </Tabs.Tab>
          <Tabs.Tab value="items" color={Object.keys(errors.items).length ? 'red' : undefined}>
            Items <Text span fw={600}>{formatMoney(itemsTotal(draft.items))}</Text>
          </Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel value="expenses" pt="xs">
          <LineGrid
            label="Expense lines"
            rows={draft.expenses}
            onChange={(expenses) => update({ expenses })}
            columns={expenseColumns}
            rowKey={(r) => r.key}
            isBlank={isBlankExpense}
            makeBlank={blankExpense}
            errors={errors.expenses}
            enterBehavior={enterBehavior}
            onEnterSave={onEnterSave}
            mobile={mobile}
            minVisibleRows={5}
          />
        </Tabs.Panel>
        <Tabs.Panel value="items" pt="xs">
          <LineGrid
            label="Item lines"
            rows={draft.items}
            onChange={(rows) => update({ items: rows })}
            columns={itemColumns}
            rowKey={(r) => r.key}
            isBlank={isBlankItem}
            makeBlank={blankItemRow}
            errors={errors.items}
            enterBehavior={enterBehavior}
            onEnterSave={onEnterSave}
            mobile={mobile}
            minVisibleRows={5}
          />
        </Tabs.Panel>
      </Tabs>

      <FormActionBar mobile={mobile}>
        {!mobile && saved && (
          <ShortcutButton variant="subtle" color="red" command="record.delete" onClick={() => void onDelete()}>
            Delete
          </ShortcutButton>
        )}
        {!mobile && (
          <ShortcutButton variant="default" command="txn.journal" onClick={() => setJournalFor(draftToTxn(draft))}>
            Journal
          </ShortcutButton>
        )}
        {!mobile && (
          <ShortcutButton
            variant="default"
            onClick={() => reset(saved ? draftFromTxn(saved) : newCheckDraft(draft.date, draft.account_id))}
            disabled={!dirty}
          >
            {saved ? 'Revert' : 'Clear'}
          </ShortcutButton>
        )}
        <ShortcutButton variant="default" command="form.saveNew" loading={save.isPending} onClick={() => void doSave('new')}>
          Save &amp; New
        </ShortcutButton>
        <ShortcutButton command="form.saveClose" loading={save.isPending} onClick={() => void doSave('close')}>
          Save &amp; Close
        </ShortcutButton>
      </FormActionBar>

      <ConflictModal
        opened={!!conflict}
        what="check"
        onCancel={() => setConflict(null)}
        onReload={() => {
          setConflict(null);
          setBaseline(draftFingerprint(draft));
          void qc.invalidateQueries({ queryKey: qk.txn(co, draft.id) });
        }}
        onOverwrite={() => {
          setConflict(null);
          void doSave('close', { overwrite: true });
        }}
      />
      <PostingsModal txn={journalFor} onClose={() => setJournalFor(null)} />
      <HistoryModal txnId={historyFor} onClose={() => setHistoryFor(null)} />
    </div>
  );
}
