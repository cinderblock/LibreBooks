// Pay Bills: every open bill in one grid, filtered by due date; tick the ones to
// pay, take a discount or a vendor credit, choose the account and the date, and
// one bill payment per vendor is written (BillPaymentCheck from a bank,
// BillPaymentCreditCard from a card). Not a single transaction, so not a
// TxnFormSpec form: it runs the saves itself and reports what it wrote.

import { Alert, Button, Card, Checkbox, Group, Modal, SegmentedControl, Stack, Table, Text, TextInput } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex, useCompanyInfo, useEntityIndex, useNextNumber, useTerms, useTxns } from '~/api/hooks';
import { invalidateTxnDerived } from '~/api/keys';
import type { Txn, TxnSummary } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill } from '~/inputs/pickers';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { addDays, formatDateShort, todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { FormActionBar, ShortcutButton } from './common';
import { billRows, buildBillPayments, cashFor, filterRows, open as openOf, payBillsTotals, selectAll, selectRow, setRowAmount, setRowCredit, setRowDiscount, suggestedDiscount, validatePayBills, type BillRow } from './billPayments';
import classes from './forms.module.css';

const LAST_ACCOUNT_KEY = 'librebooks.payBills.account';

export function PayBills() {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  const win = useWindow();
  const openWindow = useOpenWindow();
  const mobile = useIsMobile();
  const { data: info } = useCompanyInfo();
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const { data: terms = [] } = useTerms();
  const bills = useTxns({ types: ['Bill'], open_only: true, limit: 2000 });
  const credits = useTxns({ types: ['VendorCredit', 'BillPaymentCheck', 'BillPaymentCreditCard'], open_only: true, limit: 2000 });
  const [rows, setRows] = useState<BillRow[]>([]);
  const [showAll, setShowAll] = useState(false);
  const [dueBy, setDueBy] = useState<string | null>(addDays(todayIso(), 7));
  const [date, setDate] = useState(todayIso());
  const [accountId, setAccountId] = useState<string | null>(() => localStorage.getItem(LAST_ACCOUNT_KEY) ?? info?.preferences.default_check_account ?? null);
  const [method, setMethod] = useState<'print' | 'assign'>('assign');
  const [number, setNumber] = useState('');
  const [discountAccount, setDiscountAccount] = useState<string | null>(null);
  const [memo, setMemo] = useState('');
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<Txn[] | null>(null);

  useEffect(() => {
    if (bills.data) setRows((prev) => billRows(bills.data!, prev));
  }, [bills.data]);
  useEffect(() => {
    if (!accountId && info?.preferences.default_check_account) setAccountId(info.preferences.default_check_account);
  }, [info]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => win.setTitle('Pay Bills'), []); // eslint-disable-line react-hooks/exhaustive-deps

  const totals = payBillsTotals(rows);
  useEffect(() => win.setDirty(totals.selected > 0), [totals.selected]); // eslint-disable-line react-hooks/exhaustive-deps

  const account = accountId ? accounts.get(accountId) : undefined;
  const next = useNextNumber('BillPaymentCheck', accountId, method === 'assign' && account?.account_type !== 'CreditCard');
  useEffect(() => {
    if (next.data && !number) setNumber(next.data);
  }, [next.data]); // eslint-disable-line react-hooks/exhaustive-deps

  const shown = useMemo(() => filterRows(rows, showAll ? null : dueBy), [rows, showAll, dueBy]);
  const current = rows.find((r) => r.bill.id === selected);
  const vendorCredits = useMemo(() => (credits.data ?? []).filter((c) => current && c.entity_id === current.bill.entity_id && (c.open_balance ?? 0) > 0), [credits.data, current]);
  const vendorTerms = (b: TxnSummary) => {
    const v = b.entity_id ? entities.get(b.entity_id) : undefined;
    return v?.terms_id ? terms.find((t) => t.id === v.terms_id) : undefined;
  };

  const pay = async () => {
    const problem = validatePayBills(rows, { account_id: accountId ?? undefined, discount_account_id: discountAccount ?? undefined });
    if (problem || !account) {
      setError(problem ?? 'Choose the account to pay from.');
      return;
    }
    const txns = buildBillPayments(rows, {
      date,
      account_id: account.id,
      account_type: account.account_type,
      number: account.account_type === 'CreditCard' || method === 'print' ? null : number.trim() || null,
      discount_account_id: discountAccount,
      memo: memo.trim() || null,
    });
    setBusy(true);
    setError(null);
    const saved: Txn[] = [];
    try {
      for (const t of txns) saved.push((await api.createTxn(co, t)).txn);
      localStorage.setItem(LAST_ACCOUNT_KEY, account.id);
      invalidateTxnDerived(qc, co);
      setDone(saved);
      setRows((r) => selectAll(r, false));
    } catch (e) {
      invalidateTxnDerived(qc, co);
      setError(`${saved.length ? `${saved.length} payment(s) were saved before this: ` : ''}${describeError(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const enabled = win.isActiveNow;
  useShortcut('form.saveClose', () => void pay(), { enabled });

  const row = (r: BillRow) => {
    const id = r.bill.id;
    const vendor = r.bill.entity_name ?? '';
    const disc = suggestedDiscount(r.bill, vendorTerms(r.bill), date);
    if (mobile) {
      return (
        <Card key={id} withBorder padding="sm" onClick={() => setSelected(id)} data-selected={selected === id || undefined}>
          <Group justify="space-between" wrap="nowrap">
            <Checkbox label={`${vendor} · ${r.bill.number ?? ''}`} checked={r.selected} onChange={(e) => setRows(selectRow(rows, id, e.currentTarget.checked))} />
            <Text size="sm" className={classes.total}>
              {formatMoney(openOf(r.bill))}
            </Text>
          </Group>
          <Text size="xs" c="dimmed">
            Due {r.bill.due_date ? formatDateShort(r.bill.due_date) : '—'}
            {disc ? ` · discount ${formatMoney(disc)} available` : ''}
          </Text>
          {r.selected && <MoneyField label="Amount to pay" size="xs" value={r.amount} onChange={(v) => setRows(setRowAmount(rows, id, v ?? 0))} />}
        </Card>
      );
    }
    return (
      <Table.Tr key={id} onClick={() => setSelected(id)} data-selected={selected === id || undefined} className={classes.selectable}>
        <Table.Td>
          <Checkbox aria-label={`Pay ${vendor} ${r.bill.number ?? ''}`} checked={r.selected} onChange={(e) => setRows(selectRow(rows, id, e.currentTarget.checked))} />
        </Table.Td>
        <Table.Td>{r.bill.due_date ? formatDateShort(r.bill.due_date) : ''}</Table.Td>
        <Table.Td>{vendor}</Table.Td>
        <Table.Td>{r.bill.number}</Table.Td>
        <Table.Td ta="right">{formatMoney(openOf(r.bill))}</Table.Td>
        <Table.Td ta="right">{disc ? formatMoney(disc) : ''}</Table.Td>
        <Table.Td ta="right">{r.discount ? formatMoney(r.discount) : ''}</Table.Td>
        <Table.Td ta="right">{r.credit_amount ? formatMoney(r.credit_amount) : ''}</Table.Td>
        <Table.Td>
          <MoneyField variant="cell" size="xs" blankZero value={r.amount} onChange={(v) => setRows(setRowAmount(rows, id, v ?? 0))} aria-label={`Amount to pay ${vendor} ${r.bill.number ?? ''}`} />
        </Table.Td>
      </Table.Tr>
    );
  };

  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid="form-pay-bills">
      <Group gap="md" align="flex-end" wrap="wrap">
        <SegmentedControl
          size="xs"
          data={[
            { value: 'due', label: 'Due on or before' },
            { value: 'all', label: 'Show all bills' },
          ]}
          value={showAll ? 'all' : 'due'}
          onChange={(v) => setShowAll(v === 'all')}
        />
        {!showAll && <DateField size="xs" label="Due on or before" value={dueBy} onChange={setDueBy} aria-label="Due on or before" />}
        <Button size="xs" variant="default" onClick={() => setRows(selectAll(rows, true))} disabled={!shown.length}>
          Select All
        </Button>
        <Button size="xs" variant="default" onClick={() => setRows(selectAll(rows, false))} disabled={!totals.selected}>
          Clear
        </Button>
      </Group>
      {bills.error && <Alert color="red">{describeError(bills.error)}</Alert>}
      {mobile ? (
        <Stack gap="xs">{shown.map(row)}</Stack>
      ) : (
        <Table fz="sm" withTableBorder highlightOnHover aria-label="Bills to pay">
          <Table.Thead>
            <Table.Tr>
              <Table.Th w={36} />
              <Table.Th>Date Due</Table.Th>
              <Table.Th>Vendor</Table.Th>
              <Table.Th>Ref. No.</Table.Th>
              <Table.Th ta="right">Amt. Due</Table.Th>
              <Table.Th ta="right">Disc. Avail.</Table.Th>
              <Table.Th ta="right">Disc. Used</Table.Th>
              <Table.Th ta="right">Credits Used</Table.Th>
              <Table.Th w={120}>Amt. To Pay</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>{shown.map(row)}</Table.Tbody>
          <Table.Tfoot>
            <Table.Tr>
              <Table.Th colSpan={4}>{totals.selected} selected</Table.Th>
              <Table.Th ta="right">{formatMoney(totals.open)}</Table.Th>
              <Table.Th />
              <Table.Th ta="right">{formatMoney(totals.discount)}</Table.Th>
              <Table.Th ta="right">{formatMoney(totals.credit)}</Table.Th>
              <Table.Th ta="right">{formatMoney(totals.cash)}</Table.Th>
            </Table.Tr>
          </Table.Tfoot>
        </Table>
      )}
      {!shown.length && !bills.isLoading && (
        <Text c="dimmed" size="sm">
          No open bills{showAll ? '' : ' due by that date'}.
        </Text>
      )}

      {current && (
        <Card withBorder padding="sm">
          <Text fw={600} size="sm" mb="xs">
            Discount & credit information for {current.bill.entity_name} {current.bill.number ?? ''}
          </Text>
          <Group align="flex-end" gap="md" wrap="wrap">
            <Text size="sm" c="dimmed">
              Terms: {vendorTerms(current.bill)?.name ?? 'none'} · suggested discount {formatMoney(suggestedDiscount(current.bill, vendorTerms(current.bill), date))}
            </Text>
            <MoneyField label="Discount to take" size="xs" blankZero value={current.discount} onChange={(v) => setRows(setRowDiscount(rows, current.bill.id, v ?? 0))} aria-label="Discount to take" />
            <Button size="xs" variant="default" onClick={() => setRows(setRowDiscount(rows, current.bill.id, suggestedDiscount(current.bill, vendorTerms(current.bill), date)))}>
              Take suggested
            </Button>
            {vendorCredits.length > 0 && (
              <Group gap="xs" align="flex-end">
                <Text size="sm" c="dimmed">
                  Credits: {vendorCredits.map((c) => `${c.number ?? c.txn_type} ${formatMoney(c.open_balance ?? 0)}`).join(', ')}
                </Text>
                <Button
                  size="xs"
                  variant="default"
                  onClick={() => {
                    const c = vendorCredits[0]!;
                    setRows(setRowCredit(rows, current.bill.id, c.id, Math.min(c.open_balance ?? 0, cashFor({ ...current, credit_amount: 0, credit_txn_id: null }))));
                  }}
                >
                  Apply {formatMoney(Math.min(vendorCredits[0]!.open_balance ?? 0, openOf(current.bill)))} credit
                </Button>
                {current.credit_amount > 0 && (
                  <Button size="xs" variant="subtle" onClick={() => setRows(setRowCredit(rows, current.bill.id, null, 0))}>
                    Remove credit
                  </Button>
                )}
              </Group>
            )}
          </Group>
        </Card>
      )}

      <Card withBorder padding="sm">
        <Text fw={600} size="sm" mb="xs">
          Payment
        </Text>
        <Group align="flex-end" gap="md" wrap="wrap">
          <AccountQuickFill label="Account" types={['Bank', 'OtherCurrentAsset', 'CreditCard']} value={accountId} onChange={setAccountId} allowAddNew={false} aria-label="Pay from account" style={{ flex: '0 1 260px' }} />
          <DateField label="Date" value={date} onChange={(d) => d && setDate(d)} aria-label="Payment date" />
          {account?.account_type !== 'CreditCard' && (
            <>
              <SegmentedControl
                size="xs"
                data={[
                  { value: 'assign', label: 'Assign check number' },
                  { value: 'print', label: 'To be printed' },
                ]}
                value={method}
                onChange={(v) => setMethod(v as 'print' | 'assign')}
              />
              {method === 'assign' && <TextInput label="First check no." size="xs" value={number} onChange={(e) => setNumber(e.currentTarget.value)} w={110} aria-label="First check number" />}
            </>
          )}
          {totals.discount > 0 && (
            <AccountQuickFill label="Discount account" types={['Income', 'Expense', 'OtherIncome', 'OtherExpense', 'CostOfGoodsSold']} value={discountAccount} onChange={setDiscountAccount} allowAddNew={false} aria-label="Discount account" />
          )}
          <TextInput label="Memo" size="xs" value={memo} onChange={(e) => setMemo(e.currentTarget.value)} aria-label="Memo" />
          <Text size="sm" ml="auto">
            Ending balance{' '}
            <Text span fw={600}>
              {account ? formatMoney(0 - totals.cash) : '—'}
            </Text>{' '}
            change
          </Text>
        </Group>
      </Card>
      {error && (
        <Alert color="red" role="alert">
          {error}
        </Alert>
      )}
      <FormActionBar mobile={mobile}>
        <Button variant="default" onClick={() => win.close()}>
          Cancel
        </Button>
        <ShortcutButton command="form.saveClose" loading={busy} onClick={() => void pay()} disabled={!totals.selected}>
          Pay Selected Bills
        </ShortcutButton>
      </FormActionBar>
      <Modal opened={!!done} onClose={() => setDone(null)} title="Payment Summary" centered>
        <Stack gap="sm">
          <Text size="sm">
            {done?.length ?? 0} payment{done?.length === 1 ? '' : 's'} written from {account?.full_name ?? 'the account'} on {formatDateShort(date)}.
          </Text>
          <Table fz="sm">
            <Table.Tbody>
              {(done ?? []).map((t) => (
                <Table.Tr key={t.id} className={classes.selectable} onClick={() => openWindow(paths.txn(t.id))}>
                  <Table.Td>{t.entity_id ? entities.get(t.entity_id)?.full_name : ''}</Table.Td>
                  <Table.Td>{t.number ?? (t.is_to_be_printed ? 'To Print' : '')}</Table.Td>
                  <Table.Td ta="right">{formatMoney(t.amount)}</Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setDone(null)}>
              Pay More Bills
            </Button>
            <Button
              onClick={() => {
                notifications.show({ message: `${done?.length ?? 0} bill payment(s) saved.`, color: 'teal' });
                win.close({ force: true });
              }}
            >
              Done
            </Button>
          </Group>
        </Stack>
      </Modal>
    </div>
  );
}
