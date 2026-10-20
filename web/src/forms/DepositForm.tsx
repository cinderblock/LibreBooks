// Make Deposits: the payments waiting in Undeposited Funds (picked from a list),
// any other lines deposited straight to an account, and cash back, into one
// bank account.

import { Button, Checkbox, Group, Modal, Stack, Table, Text, TextInput } from '@mantine/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useAccounts, useCompanyInfo, useEntityIndex, usePaymentMethods, useUndeposited } from '~/api/hooks';
import type { SaveOutcome, Txn, TxnSummary } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { LineGrid, type GridColumn } from '~/inputs/LineGrid';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, ClassQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useDateWarning } from '~/inputs/useDateWarning';
import { formatDateShort, todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { paths } from '~/windows/paths';
import { gridErrors, type TxnFormSpec } from './model';
import {
  addPayments,
  blankDepositRow,
  depositDraftFromTxn,
  depositDraftToTxn,
  depositFingerprint,
  depositServerErrors,
  depositSubtotal,
  depositTotal,
  isBlankDeposit,
  isDepositEmpty,
  newDepositDraft,
  nextDepositDraft,
  validateDeposit,
  type DepositDraft,
  type DepositRow,
} from './depositDraft';
import { FormBadges, TxnFormActions, useTxnForm } from './useTxnForm';
import classes from './forms.module.css';

const LAST_ACCOUNT_KEY = 'librebooks.deposit.account';

function spec(defaultAccount: () => string | null): TxnFormSpec<DepositDraft> {
  return {
    noun: 'deposit',
    newDraft: () => newDepositDraft(todayIso(), defaultAccount()),
    fromTxn: depositDraftFromTxn,
    toTxn: depositDraftToTxn,
    validate: validateDeposit,
    isEmpty: isDepositEmpty,
    fingerprint: depositFingerprint,
    serverErrors: depositServerErrors,
    nextDraft: nextDepositDraft,
    newPath: paths.form('deposit'),
    savedMessage: (out: SaveOutcome) => `Deposit of ${formatMoney(out.total, { symbol: true })} saved.`,
    siblings: (d) => ({ types: ['Deposit'], ...(d.account_id ? { account_id: d.account_id } : {}) }),
    title: (_d, saved) => (saved ? `Deposit ${formatDateShort(saved.date)}` : 'Make Deposits'),
  };
}

export function DepositForm({ saved, initial }: { saved: Txn | null; initial: Txn | null }) {
  const { data: info } = useCompanyInfo();
  const { data: accountList = [] } = useAccounts();
  const entities = useEntityIndex();
  const { data: methods = [] } = usePaymentMethods();
  const firstField = useRef<HTMLInputElement>(null);
  const defaultAccount = () => localStorage.getItem(LAST_ACCOUNT_KEY) ?? info?.preferences.default_deposit_account ?? null;
  const form = useTxnForm({ spec: useMemo(() => spec(defaultAccount), [info]), saved, initial, onSavedNew: () => firstField.current?.focus() }); // eslint-disable-line react-hooks/exhaustive-deps
  const { draft, update, errors, mobile, enterBehavior } = form;
  const uf = useMemo(() => accountList.find((a) => a.role === 'UndepositedFunds'), [accountList]);
  const undeposited = useUndeposited(!!uf);
  const [picking, setPicking] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!draft.account_id && draft.edit_seq === 0 && !form.dirty) {
      const d = defaultAccount();
      if (d) update({ account_id: d });
    }
  }, [info]); // eslint-disable-line react-hooks/exhaustive-deps

  // A new deposit opens on the payments to deposit, as QBD does, when there are any.
  const offered = useRef(false);
  useEffect(() => {
    if (!saved && !initial && !offered.current && undeposited.data?.length && draft.rows.every(isBlankDeposit)) {
      offered.current = true;
      setPicked(new Set(undeposited.data.map((p) => p.id)));
      setPicking(true);
    }
  }, [undeposited.data]); // eslint-disable-line react-hooks/exhaustive-deps

  const dateWarning = useDateWarning(draft.date);
  const useClasses = info?.preferences.use_class_tracking ?? false;
  const onEnterSave = useCallback(() => void form.save('close'), []); // eslint-disable-line react-hooks/exhaustive-deps
  const linked = new Set(draft.rows.map((r) => r.linked_txn_id).filter(Boolean));
  const available = (undeposited.data ?? []).filter((p) => !linked.has(p.id));

  const columns = useMemo<GridColumn<DepositRow>[]>(
    () => [
      {
        key: 'from',
        header: 'Received From',
        width: '20%',
        render: (c) => (
          <NameQuickFill variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" value={c.row.entity_id} onChange={(id) => c.update((r) => ({ ...r, entity_id: id }))} onCommit={c.onCommit} disabled={!!c.row.linked_txn_id} inputProps={c.cellProps} />
        ),
      },
      {
        key: 'account',
        header: 'From Account',
        width: '22%',
        render: (c) => (
          <AccountQuickFill variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" value={c.row.account_id} onChange={(id) => c.update((r) => ({ ...r, account_id: id }))} onCommit={c.onCommit} error={c.error} disabled={!!c.row.linked_txn_id} inputProps={c.cellProps} />
        ),
      },
      {
        key: 'memo',
        header: 'Memo',
        mobile: 'full',
        render: (c) => (
          <TextInput size="xs" variant={c.variant === 'cell' ? 'unstyled' : 'default'} label={c.variant === 'default' ? c.label : undefined} classNames={c.variant === 'cell' ? { input: 'lb-cell-text' } : undefined} value={c.row.memo} onChange={(e) => { const memo = e.currentTarget.value; c.update((r) => ({ ...r, memo })); }} onFocus={(e) => e.currentTarget.select()} {...c.cellProps} />
        ),
      },
      {
        key: 'chk',
        header: 'Chk No.',
        width: 80,
        mobile: 'half',
        render: (c) => (
          <TextInput size="xs" variant={c.variant === 'cell' ? 'unstyled' : 'default'} label={c.variant === 'default' ? c.label : undefined} classNames={c.variant === 'cell' ? { input: 'lb-cell-text' } : undefined} value={c.row.check_number} onChange={(e) => { const check_number = e.currentTarget.value; c.update((r) => ({ ...r, check_number })); }} onFocus={(e) => e.currentTarget.select()} {...c.cellProps} />
        ),
      },
      {
        key: 'method',
        header: 'Pmt Meth.',
        width: 100,
        mobile: 'half',
        render: (c) => (
          <select
            className="lb-cell-text"
            style={{ width: '100%', border: 0, background: 'transparent' }}
            value={c.row.payment_method_id ?? ''}
            onChange={(e) => {
              const payment_method_id = e.currentTarget.value || null;
              c.update((r) => ({ ...r, payment_method_id }));
            }}
            {...c.cellProps}
          >
            <option value="">—</option>
            {methods.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        ),
      },
      ...(useClasses
        ? [
            {
              key: 'class',
              header: 'Class',
              width: '12%',
              mobile: 'half' as const,
              render: (c) => <ClassQuickFill variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" value={c.row.class_id} onChange={(id) => c.update((r) => ({ ...r, class_id: id }))} onCommit={c.onCommit} inputProps={c.cellProps} />,
            } satisfies GridColumn<DepositRow>,
          ]
        : []),
      {
        key: 'amount',
        header: 'Amount',
        width: 110,
        align: 'right',
        mobile: 'half',
        render: (c) => (
          <MoneyField variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" blankZero value={c.row.amount} onChange={(v) => c.update((r) => ({ ...r, amount: v }))} error={c.error} disabled={!!c.row.linked_txn_id} inputProps={c.cellProps} />
        ),
        footer: <span className={classes.total}>{formatMoney(depositSubtotal(draft))}</span>,
      },
    ],
    [methods, useClasses, draft], // eslint-disable-line react-hooks/exhaustive-deps
  );

  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid="form-deposit">
      {form.banner}
      <Group align="flex-end" gap="md" wrap="wrap">
        <AccountQuickFill
          label="Deposit To"
          types={['Bank', 'OtherCurrentAsset']}
          value={draft.account_id}
          onChange={(id) => {
            update({ account_id: id });
            if (id) localStorage.setItem(LAST_ACCOUNT_KEY, id);
          }}
          error={errors.header.account}
          inputRef={firstField}
          autoFocus={!saved && !mobile}
          allowAddNew={false}
          style={{ flex: mobile ? '1 1 100%' : '0 1 300px' }}
          aria-label="Deposit to"
        />
        <DateField label="Date" value={draft.date} onChange={(date) => date && update({ date })} error={errors.header.date} warning={dateWarning} aria-label="Date" />
        <TextInput label="Memo" value={draft.memo} onChange={(e) => { const memo = e.currentTarget.value; update({ memo }); }} aria-label="Memo" />
        <Button variant="light" onClick={() => setPicking(true)} disabled={!uf || available.length === 0}>
          Payments to deposit{available.length ? ` (${available.length})` : ''}
        </Button>
        <FormBadges saved={saved} />
      </Group>
      {form.formErrors}
      <LineGrid label="Deposit lines" rows={draft.rows} onChange={(rows) => update({ rows })} columns={columns} rowKey={(r) => r.key} isBlank={isBlankDeposit} makeBlank={blankDepositRow} errors={gridErrors(errors, 'rows')} enterBehavior={enterBehavior} onEnterSave={onEnterSave} mobile={mobile} minVisibleRows={4} />
      <Group align="flex-end" gap="md" wrap="wrap" justify="space-between">
        <Group align="flex-end" gap="sm" wrap="wrap">
          <AccountQuickFill label="Cash back goes to" types={['Bank', 'OtherCurrentAsset', 'Expense', 'Equity']} value={draft.cash_back_account_id} onChange={(cash_back_account_id) => update({ cash_back_account_id })} error={errors.header.cash_back} allowAddNew={false} aria-label="Cash back account" />
          <TextInput label="Cash back memo" value={draft.cash_back_memo} onChange={(e) => { const cash_back_memo = e.currentTarget.value; update({ cash_back_memo }); }} aria-label="Cash back memo" />
          <MoneyField label="Cash back amount" value={draft.cash_back_amount} onChange={(cash_back_amount) => update({ cash_back_amount })} aria-label="Cash back amount" />
        </Group>
        <div className={classes.totals} data-testid="deposit-totals">
          <span className={classes.label}>Deposit subtotal</span>
          <span className={classes.value}>{formatMoney(depositSubtotal(draft))}</span>
          <span className={classes.label}>Cash back</span>
          <span className={classes.value}>{formatMoney(draft.cash_back_amount ?? 0)}</span>
          <span className={`${classes.label} ${classes.grand}`}>Deposit total</span>
          <span className={`${classes.value} ${classes.grand}`} aria-label="Deposit total">
            {formatMoney(depositTotal(draft))}
          </span>
        </div>
      </Group>
      <TxnFormActions form={form} />
      {form.chrome}
      <Modal opened={picking} onClose={() => setPicking(false)} title="Payments to Deposit" size="lg" centered>
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            Select the payments in {uf?.full_name ?? 'Undeposited Funds'} to put on this deposit.
          </Text>
          <Table fz="sm" highlightOnHover>
            <Table.Thead>
              <Table.Tr>
                <Table.Th w={36}>
                  <Checkbox aria-label="Select all" checked={available.length > 0 && available.every((p) => picked.has(p.id))} onChange={(e) => setPicked(e.currentTarget.checked ? new Set(available.map((p) => p.id)) : new Set())} />
                </Table.Th>
                <Table.Th>Date</Table.Th>
                <Table.Th>Type</Table.Th>
                <Table.Th>No.</Table.Th>
                <Table.Th>Name</Table.Th>
                <Table.Th ta="right">Amount</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {available.map((p: TxnSummary) => (
                <Table.Tr key={p.id}>
                  <Table.Td>
                    <Checkbox aria-label={`Deposit ${p.entity_name ?? ''} ${formatMoney(p.amount)}`} checked={picked.has(p.id)} onChange={(e) => setPicked((s) => { const n = new Set(s); if (e.currentTarget.checked) n.add(p.id); else n.delete(p.id); return n; })} />
                  </Table.Td>
                  <Table.Td>{formatDateShort(p.date)}</Table.Td>
                  <Table.Td>{TXN_LABEL[p.txn_type]}</Table.Td>
                  <Table.Td>{p.number}</Table.Td>
                  <Table.Td>{p.entity_name ?? (p.entity_id ? entities.get(p.entity_id)?.full_name : '')}</Table.Td>
                  <Table.Td ta="right">{formatMoney(p.amount)}</Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
          {available.length === 0 && <Text c="dimmed" size="sm">Nothing is waiting to be deposited.</Text>}
          <Group justify="space-between">
            <Text size="sm">
              {picked.size} selected · {formatMoney(available.filter((p) => picked.has(p.id)).reduce((s, p) => s + p.amount, 0))}
            </Text>
            <Group>
              <Button variant="default" onClick={() => setPicking(false)}>
                Cancel
              </Button>
              <Button
                onClick={() => {
                  if (uf) update((d) => addPayments(d, available.filter((p) => picked.has(p.id)), uf.id));
                  setPicking(false);
                  setPicked(new Set());
                }}
                disabled={!picked.size}
              >
                OK
              </Button>
            </Group>
          </Group>
        </Stack>
      </Modal>
    </div>
  );
}
