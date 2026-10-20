// Receive Payments, and the bill payment check / card payment that Pay Bills
// writes (opened from a register or the Vendor Center to edit). A header saying
// who paid, how much, how and into (or from) which account, over the name's open
// invoices or bills: tick one to pay it, or type the amount and let auto-apply
// take the oldest first; discounts and credits reduce the cash each line needs.

import { Button, Card, Checkbox, Group, Popover, Select, Stack, Table, Text, TextInput } from '@mantine/core';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useAccountIndex, useCompanyInfo, useEntityIndex, useNextNumber, useOpenItems, usePaymentMethods, useTxns } from '~/api/hooks';
import type { SaveOutcome, Txn, TxnSummary } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useDateWarning } from '~/inputs/useDateWarning';
import { formatDateShort, todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { paths } from '~/windows/paths';
import { gridErrors, type TxnFormSpec } from './model';
import {
  appliedCash,
  autoApply,
  availableFor,
  CREDIT_TYPES,
  isPaymentEmpty,
  newPaymentDraft,
  nextPaymentDraft,
  paymentDraftFromTxn,
  paymentDraftToTxn,
  paymentFingerprint,
  paymentKind,
  paymentServerErrors,
  setPaymentAmount,
  setRowAmount,
  setRowCredit,
  setRowDiscount,
  syncTargets,
  TARGET_TYPES,
  toggleRow,
  unapplied,
  validatePayment,
  type AppRow,
  type PaymentDraft,
  type PaymentType,
} from './paymentDraft';
import { FormBadges, TxnFormActions, useTxnForm } from './useTxnForm';
import classes from './forms.module.css';

export const PAYMENT_FORM_SLUG: Record<PaymentType, string> = { ReceivePayment: 'receive-payment', BillPaymentCheck: 'pay-bills', BillPaymentCreditCard: 'pay-bills' };
const NOUN: Record<PaymentType, string> = { ReceivePayment: 'payment', BillPaymentCheck: 'bill payment', BillPaymentCreditCard: 'bill payment' };
const LAST_ACCOUNT_KEY = 'librebooks.payment.account';

function spec(type: PaymentType, defaultAccount: () => string | null): TxnFormSpec<PaymentDraft> {
  return {
    noun: (d) => NOUN[d.txn_type],
    newDraft: () => newPaymentDraft(type, todayIso(), defaultAccount()),
    fromTxn: paymentDraftFromTxn,
    toTxn: paymentDraftToTxn,
    validate: validatePayment,
    isEmpty: isPaymentEmpty,
    fingerprint: paymentFingerprint,
    serverErrors: paymentServerErrors,
    nextDraft: nextPaymentDraft,
    newPath: paths.form(PAYMENT_FORM_SLUG[type]),
    savedMessage: (out: SaveOutcome) => `${out.txn.txn_type === 'ReceivePayment' ? 'Payment' : 'Bill payment'} of ${formatMoney(out.total, { symbol: true })} saved.`,
    siblings: (d) => ({ types: [d.txn_type] }),
    title: (d, saved) => (saved ? `${TXN_LABEL[d.txn_type]}${saved.number ? ` ${saved.number}` : ''}` : d.txn_type === 'ReceivePayment' ? 'Receive Payments' : 'Bill Payment'),
  };
}

export function PaymentForm({ type, saved, initial }: { type: PaymentType; saved: Txn | null; initial: Txn | null }) {
  const { data: info } = useCompanyInfo();
  const prefs = info?.preferences;
  const entities = useEntityIndex();
  const accounts = useAccountIndex();
  const { data: methods = [] } = usePaymentMethods();
  const firstField = useRef<HTMLInputElement>(null);
  const kind = paymentKind(type);
  const defaultAccount = () => (kind === 'receivable' ? (prefs?.use_undeposited_funds === false ? (prefs.default_deposit_account ?? null) : null) : localStorage.getItem(LAST_ACCOUNT_KEY));
  const form = useTxnForm({ spec: useMemo(() => spec(type, defaultAccount), [type, prefs]), saved, initial, onSavedNew: () => firstField.current?.focus() }); // eslint-disable-line react-hooks/exhaustive-deps
  const { draft, update, errors, mobile } = form;
  const autoPref = prefs?.auto_apply_payments ?? true;
  const [discountAccount, setDiscountAccount] = useState<string | null>(null);

  // The name's invoices (or bills), all of them: a saved payment's targets may be paid off by it.
  const targets = useTxns({ types: TARGET_TYPES[kind], entity_id: draft.entity_id ?? undefined, include_jobs: true, limit: 500 }, !!draft.entity_id);
  const openItems = useOpenItems(draft.entity_id, kind);
  const credits = useMemo(() => (openItems.data ?? []).filter((t) => CREDIT_TYPES[kind].includes(t.txn_type) && t.id !== draft.id && (t.open_balance ?? 0) > 0), [openItems.data, kind, draft.id]);
  useEffect(() => {
    if (targets.data) update((d) => syncTargets(d, targets.data!));
  }, [targets.data]); // eslint-disable-line react-hooks/exhaustive-deps

  // Bill payment checks number themselves per bank account.
  const wantsAuto = draft.edit_seq === 0 && draft.numberIsAuto && draft.txn_type === 'BillPaymentCheck';
  const next = useNextNumber('BillPaymentCheck', draft.account_id, wantsAuto);
  useEffect(() => {
    if (next.data !== undefined && wantsAuto) update((d) => (d.numberIsAuto ? { ...d, number: next.data ?? '' } : d));
  }, [next.data]); // eslint-disable-line react-hooks/exhaustive-deps

  const dateWarning = useDateWarning(draft.date);
  const rowErrors = gridErrors(errors, 'rows');
  const account = draft.account_id ? accounts.get(draft.account_id) : undefined;
  const name = draft.entity_id ? entities.get(draft.entity_id) : undefined;
  const applied = appliedCash(draft);
  const left = unapplied(draft);
  const discounts = draft.rows.reduce((s, r) => s + r.discount, 0);
  const creditsUsed = draft.rows.reduce((s, r) => s + r.credit_amount, 0);

  const onName = (id: string | null) => update((d) => ({ ...d, entity_id: id, rows: id === d.entity_id ? d.rows : [] }));

  const header = (
    <>
      <Group align="flex-end" gap="md" wrap="wrap">
        <FormBadges saved={saved} />
        {kind === 'receivable' && name && (
          <Text size="sm" c="dimmed">
            Customer balance{' '}
            <Text span fw={600} c="var(--mantine-color-text)">
              {formatMoney((targets.data ?? []).reduce((s, t) => s + (t.open_balance ?? 0), 0))}
            </Text>
          </Text>
        )}
      </Group>
      <div className={mobile ? undefined : classes.headerGrid}>
        <NameQuickFill
          label={kind === 'receivable' ? 'Received From' : 'Vendor'}
          kinds={kind === 'receivable' ? ['Customer'] : ['Vendor']}
          value={draft.entity_id}
          onChange={onName}
          error={errors.header.entity}
          inputRef={firstField}
          autoFocus={!saved && !mobile}
          allowAddNew={false}
          aria-label={kind === 'receivable' ? 'Received from' : 'Vendor'}
        />
        <MoneyField label="Amount" value={draft.amount} onChange={(v) => update((d) => setPaymentAmount(d, v, autoPref))} error={errors.header.amount} aria-label="Amount" size={mobile ? 'md' : 'sm'} />
        <DateField label="Date" value={draft.date} onChange={(date) => date && update({ date })} error={errors.header.date} warning={dateWarning} aria-label="Date" />
        {kind === 'receivable' ? (
          <Select
            label="Pmt. Method"
            data={methods.filter((m) => m.is_active || m.id === draft.payment_method_id).map((m) => ({ value: m.id, label: m.name }))}
            value={draft.payment_method_id}
            onChange={(payment_method_id) => update({ payment_method_id })}
            clearable
            searchable
            aria-label="Payment method"
          />
        ) : (
          <AccountQuickFill
            label="Pay From"
            types={['Bank', 'OtherCurrentAsset', 'CreditCard']}
            value={draft.account_id}
            onChange={(id) => {
              const acct = id ? accounts.get(id) : undefined;
              update((d) => ({ ...d, account_id: id, txn_type: acct?.account_type === 'CreditCard' ? 'BillPaymentCreditCard' : 'BillPaymentCheck', numberIsAuto: d.edit_seq === 0 }));
              if (id) localStorage.setItem(LAST_ACCOUNT_KEY, id);
            }}
            error={errors.header.account}
            allowAddNew={false}
            aria-label="Pay from"
          />
        )}
        <TextInput
          label={kind === 'receivable' ? 'Reference / Check No.' : 'Check No.'}
          value={draft.to_be_printed && draft.txn_type === 'BillPaymentCheck' ? 'To Print' : draft.number}
          disabled={draft.to_be_printed && draft.txn_type === 'BillPaymentCheck'}
          onChange={(e) => {
            const number = e.currentTarget.value;
            update((d) => ({ ...d, number, numberIsAuto: false }));
          }}
          error={errors.header.number}
          aria-label="Reference number"
        />
        {kind === 'receivable' ? (
          <AccountQuickFill
            label="Deposit To"
            types={['Bank', 'OtherCurrentAsset']}
            value={draft.account_id}
            onChange={(account_id) => update({ account_id })}
            placeholder="Undeposited Funds"
            allowAddNew={false}
            aria-label="Deposit to"
          />
        ) : (
          <Checkbox label="To be printed" checked={draft.to_be_printed} onChange={(e) => update({ to_be_printed: e.currentTarget.checked })} disabled={draft.txn_type !== 'BillPaymentCheck'} mb={6} />
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
        {account && (
          <Text size="sm" c="dimmed" pb={6}>
            {kind === 'receivable' ? 'Deposit to' : 'Pay from'} {account.full_name}
          </Text>
        )}
      </div>
    </>
  );

  const rowView = (r: AppRow, i: number) => {
    const avail = availableFor(draft, r);
    const err = rowErrors[i];
    const label = `${TXN_LABEL[r.summary.txn_type]} ${r.summary.number ?? ''}`.trim();
    const creditPicker = (
        <CreditPicker row={r} credits={credits} onChange={(id, amount) => update((d) => setRowCredit(d, i, id, amount))} />
    );
    if (mobile) {
      return (
        <Card key={r.target_txn_id} withBorder padding="sm" data-error={err ? true : undefined}>
          <Group justify="space-between" wrap="nowrap">
            <Checkbox label={`${label} · ${formatDateShort(r.summary.date)}`} checked={r.amount > 0} onChange={() => update((d) => toggleRow(d, i, autoPref))} />
            <Text size="sm" className={classes.total}>
              {formatMoney(avail)}
            </Text>
          </Group>
          <Group grow mt="xs">
            <MoneyField label="Payment" size="xs" blankZero value={r.amount} onChange={(v) => update((d) => setRowAmount(d, i, v))} error={err?.amount} />
            <MoneyField label="Discount" size="xs" blankZero value={r.discount} onChange={(v) => update((d) => setRowDiscount(d, i, v, discountAccount))} error={err?.discount} />
          </Group>
          {creditPicker}
        </Card>
      );
    }
    return (
      <Table.Tr key={r.target_txn_id} data-error={err ? true : undefined}>
        <Table.Td>
          <Checkbox aria-label={`Pay ${label}`} checked={r.amount > 0 || r.credit_amount > 0 || r.discount > 0} onChange={() => update((d) => toggleRow(d, i, autoPref))} />
        </Table.Td>
        <Table.Td>{formatDateShort(r.summary.date)}</Table.Td>
        <Table.Td>{label}</Table.Td>
        <Table.Td ta="right">{formatMoney(r.summary.amount)}</Table.Td>
        <Table.Td ta="right">{formatMoney(avail)}</Table.Td>
        <Table.Td>
          <MoneyField variant="cell" size="xs" blankZero value={r.discount} onChange={(v) => update((d) => setRowDiscount(d, i, v, discountAccount))} error={err?.discount} aria-label={`Discount for ${label}`} />
        </Table.Td>
        <Table.Td>{creditPicker}</Table.Td>
        <Table.Td>
          <MoneyField variant="cell" size="xs" blankZero value={r.amount} onChange={(v) => update((d) => setRowAmount(d, i, v))} error={err?.amount} aria-label={`Payment for ${label}`} />
        </Table.Td>
      </Table.Tr>
    );
  };

  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid={`form-${PAYMENT_FORM_SLUG[type]}`}>
      {form.banner}
      {header}
      {form.formErrors}
      {draft.entity_id && draft.rows.length === 0 && !targets.isLoading && (
        <Text c="dimmed" size="sm">
          Nothing open for {name?.full_name ?? 'this name'}. The amount becomes a credit to apply later.
        </Text>
      )}
      {mobile ? (
        <Stack gap="xs">{draft.rows.map(rowView)}</Stack>
      ) : (
        <Table fz="sm" withTableBorder aria-label="Open items">
          <Table.Thead>
            <Table.Tr>
              <Table.Th w={36} />
              <Table.Th>Date</Table.Th>
              <Table.Th>Number</Table.Th>
              <Table.Th ta="right">Orig. Amt.</Table.Th>
              <Table.Th ta="right">Amt. Due</Table.Th>
              <Table.Th w={110}>Discount</Table.Th>
              <Table.Th w={150}>Credits</Table.Th>
              <Table.Th w={120}>Payment</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>{draft.rows.map(rowView)}</Table.Tbody>
        </Table>
      )}
      <Group justify="space-between" align="flex-start" wrap="wrap">
        <Group gap="xs">
          <Button size="xs" variant="default" onClick={() => update((d) => autoApply(d))} disabled={!draft.amount}>
            Auto Apply
          </Button>
          <Button size="xs" variant="default" onClick={() => update((d) => ({ ...d, rows: d.rows.map((r) => ({ ...r, amount: 0 })) }))} disabled={applied === 0}>
            Un-Apply
          </Button>
          {discounts > 0 && (
            <AccountQuickFill
              label="Discount account"
              size="xs"
              types={['Income', 'Expense', 'OtherIncome', 'OtherExpense', 'CostOfGoodsSold']}
              value={discountAccount}
              onChange={(id) => {
                setDiscountAccount(id);
                update((d) => ({ ...d, rows: d.rows.map((r) => (r.discount > 0 ? { ...r, discount_account_id: id } : r)) }));
              }}
              allowAddNew={false}
              aria-label="Discount account"
            />
          )}
        </Group>
        <div className={classes.totals} data-testid="payment-totals">
          <span className={classes.label}>Applied</span>
          <span className={classes.value}>{formatMoney(applied)}</span>
          {(discounts > 0 || creditsUsed > 0) && (
            <>
              <span className={classes.label}>Discounts and credits</span>
              <span className={classes.value}>{formatMoney(discounts + creditsUsed)}</span>
            </>
          )}
          <span className={classes.label}>{left < 0 ? 'Over-applied' : 'Unapplied'}</span>
          <span className={`${classes.value} ${left < 0 ? classes.unbalanced : ''}`} aria-label="Unapplied amount">
            {formatMoney(Math.abs(left))}
          </span>
        </div>
      </Group>
      <TxnFormActions form={form} />
      {form.chrome}
    </div>
  );
}

/** Pick a credit (a credit memo, an unapplied payment) and how much of it to use on this line. */
function CreditPicker({ row, credits, onChange }: { row: AppRow; credits: TxnSummary[]; onChange(id: string | null, amount: number): void }) {
  const [open, setOpen] = useState(false);
  const [id, setId] = useState<string | null>(row.credit_txn_id);
  const [amount, setAmount] = useState<number | null>(row.credit_amount || null);
  if (!credits.length && !row.credit_amount) return <Text size="xs" c="dimmed">—</Text>;
  const chosen = credits.find((c) => c.id === id);
  return (
    <Popover opened={open} onChange={setOpen} position="bottom-end" withinPortal shadow="md" trapFocus>
      <Popover.Target>
        <Button size="compact-xs" variant={row.credit_amount ? 'light' : 'subtle'} onClick={() => setOpen((o) => !o)}>
          {row.credit_amount ? formatMoney(row.credit_amount) : 'Set credits'}
        </Button>
      </Popover.Target>
      <Popover.Dropdown
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            setOpen(false);
          }
        }}
      >
        <Stack gap="xs" w={260}>
          <Select
            label="Credit"
            size="xs"
            data={credits.map((c) => ({ value: c.id, label: `${TXN_LABEL[c.txn_type]} ${c.number ?? ''} · ${formatMoney(c.open_balance ?? 0)} open` }))}
            value={id}
            onChange={(v) => {
              setId(v);
              const c = credits.find((x) => x.id === v);
              if (c) setAmount(Math.min(c.open_balance ?? 0, row.summary.open_balance ?? c.open_balance ?? 0));
            }}
            clearable
          />
          <MoneyField label="Amount to use" size="xs" value={amount} onChange={setAmount} />
          {chosen && (
            <Text size="xs" c="dimmed">
              {formatMoney(chosen.open_balance ?? 0)} available on this credit.
            </Text>
          )}
          <Group justify="flex-end" gap="xs">
            <Button
              size="xs"
              variant="default"
              onClick={() => {
                onChange(null, 0);
                setId(null);
                setAmount(null);
                setOpen(false);
              }}
            >
              Clear
            </Button>
            <Button
              size="xs"
              onClick={() => {
                onChange(id, Math.min(amount ?? 0, chosen?.open_balance ?? amount ?? 0));
                setOpen(false);
              }}
              disabled={!id || !amount}
            >
              Done
            </Button>
          </Group>
        </Stack>
      </Popover.Dropdown>
    </Popover>
  );
}
