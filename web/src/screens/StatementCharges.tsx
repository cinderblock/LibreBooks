// Customers → Enter Statement Charges: a customer:job's A/R register (QBD's
// register for Accounts Receivable filtered to one name) with the new-charge
// line at the foot. Rows: Date · Number · Type · Memo · Amt Chrg · Amt Paid ·
// Open · Balance; ↑/↓ move, Enter (or a double-click; a tap on a phone) opens
// the transaction, Ctrl+Q QuickZooms too. The entry line records a
// `StatementCharge` with one item line (Item, Qty, Rate, Amount, Class, Due
// date from the customer's terms, Description); Enter records, Ctrl+N puts the
// caret there. Pure model: `forms/statementCharge.ts`.

import { Alert, Button, Group, Skeleton, Stack, Table, Text, TextInput, UnstyledButton } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { describeError, isApiError } from '~/api/errors';
import { useCompanyInfo, useEntityIndex, useItemIndex, useNameBalanceMap, useSaveTxn, useTerms, useTxns } from '~/api/hooks';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { ClassQuickFill, ItemQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { formatDateShort, todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL, TXN_REGISTER_CODE } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { decimalTextError } from '~/forms/purchaseLines';
import { chooseSaleItem, setSaleAmount, setSaleQuantity, setSaleRate } from '~/forms/salesLines';
import { AR_REGISTER_TYPES, arRegisterRows, blankChargeEntry, chargeEntryToTxn, isBlankCharge, type ChargeEntry, type ChargeField } from '~/forms/statementCharge';
import { dueDate } from '~/forms/terms';
import classes from './screens.module.css';

export function StatementCharges() {
  const win = useWindow();
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const { data: info } = useCompanyInfo();
  const entities = useEntityIndex();
  const items = useItemIndex();
  const { data: terms = [] } = useTerms();
  const balances = useNameBalanceMap();
  const save = useSaveTxn();
  const [customer, setCustomer] = useState<string | null>(win.desc.params.customer ?? null);
  const [entry, setEntry] = useState<ChargeEntry>(() => blankChargeEntry(todayIso()));
  const [error, setError] = useState<{ field: ChargeField; message: string } | null>(null);
  const [selected, setSelected] = useState(-1);
  const dateRef = useRef<HTMLInputElement>(null);
  const customerRef = useRef<HTMLInputElement>(null);
  const latest = useRef(entry);
  latest.current = entry;

  const who = customer ? entities.get(customer) : undefined;
  const parent = who?.parent_id ? entities.get(who.parent_id) : undefined;
  useEffect(() => win.setTitle(who ? `Statement Charges: ${who.full_name}` : 'Enter Statement Charges'), [who?.full_name]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => win.setDirty(!isBlankCharge(entry)), [entry]); // eslint-disable-line react-hooks/exhaustive-deps

  const txns = useTxns({ entity_id: customer ?? undefined, types: [...AR_REGISTER_TYPES], include_void: true, limit: 500 }, !!customer);
  const rows = useMemo(() => arRegisterRows(txns.data ?? []), [txns.data]);
  const balance = customer ? (balances.get(customer)?.receivable ?? 0) : 0;
  const useClasses = info?.preferences.use_class_tracking ?? false;

  /** The due date the customer's (or its parent's) terms give for a charge dated `date`; due on receipt without terms, as QBD. */
  const dueForName = (date: string, e: typeof who) => {
    const p = e?.parent_id ? entities.get(e.parent_id) : undefined;
    const termsId = e?.terms_id ?? p?.terms_id ?? null;
    return dueDate(termsId ? terms.find((x) => x.id === termsId) : null, date) ?? date;
  };
  const dueFor = (date: string) => dueForName(date, who);
  const onCustomer = (id: string | null) => {
    setCustomer(id);
    setSelected(-1);
    setError(null);
    const e = id ? entities.get(id) : undefined;
    setEntry((cur) => ({ ...cur, due_date: dueForName(cur.date, e) }));
  };
  const set = (patch: Partial<ChargeEntry> | ((e: ChargeEntry) => ChargeEntry)) => {
    setEntry((e) => (typeof patch === 'function' ? patch(e) : { ...e, ...patch }));
    setError(null);
  };

  const record = async () => {
    (document.activeElement as HTMLElement | null)?.blur();
    await new Promise((r) => setTimeout(r, 0));
    const r = chargeEntryToTxn(customer, latest.current);
    if (!r.ok) {
      setError({ field: r.field, message: r.message });
      (r.field === 'customer' ? customerRef : dateRef).current?.focus();
      return;
    }
    try {
      const out = await save.mutateAsync(r.txn);
      notifications.show({ message: `Statement charge for ${formatMoney(out.total, { symbol: true })} saved.`, color: 'teal' });
      setEntry((e) => blankChargeEntry(e.date, dueFor(e.date)));
      dateRef.current?.focus();
    } catch (e) {
      const msg = describeError(e);
      const code = isApiError(e) ? e.postingError?.code : undefined;
      setError({ field: code === 'MissingEntity' || code === 'WrongEntityKind' ? 'customer' : code === 'UnknownItem' || code === 'ItemWithoutAccount' ? 'item' : 'form', message: msg });
    }
  };

  const openRow = (i: number) => {
    const r = rows[i];
    if (r) open(paths.txn(r.summary.id));
  };
  const enabled = win.isActiveNow;
  useShortcut('record.new', () => dateRef.current?.focus(), { enabled });
  useShortcut('report.quick', () => (selected >= 0 ? openRow(selected) : false), { enabled });

  const onGridKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented || !rows.length) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      setSelected((s) => Math.max(0, Math.min(rows.length - 1, s + (e.key === 'ArrowDown' ? 1 : -1))));
    } else if (e.key === 'Enter' && selected >= 0) {
      e.preventDefault();
      openRow(selected);
    }
  };
  const onEntryKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      void record();
    }
  };
  const errFor = (f: ChargeField) => (error?.field === f ? error.message : undefined);

  const header = (
    <Group align="flex-end" gap="md" wrap="wrap" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0}>
      <NameQuickFill label="Customer:Job" kinds={['Customer']} value={customer} onChange={onCustomer} error={errFor('customer')} inputRef={customerRef} autoFocus={!customer && !mobile} aria-label="Customer:Job" style={{ flex: mobile ? '1 1 100%' : '0 1 360px' }} />
      {customer && (
        <Text size="sm" c="dimmed" pb={6} data-testid="ar-balance">
          Balance{' '}
          <Text span fw={700} className={classes.num} c="var(--mantine-color-text)">
            {formatMoney(balance)}
          </Text>
        </Text>
      )}
    </Group>
  );

  const entryFields = (
    <div className={mobile ? classes.chargeEntryMobile : classes.chargeEntry} data-testid="charge-entry" onKeyDown={onEntryKey}>
      <DateField label="Date" size="xs" value={entry.date} onChange={(d) => d && set((e) => ({ ...e, date: d, due_date: dueFor(d) ?? e.due_date }))} error={errFor('date')} inputRef={dateRef} aria-label="Charge date" />
      <TextInput label="Number" size="xs" value={entry.number} onChange={(e) => set({ number: e.currentTarget.value })} aria-label="Number" />
      <ItemQuickFill label="Item" side="sell" size="xs" value={entry.row.item_id} onChange={(id) => set((e) => ({ ...e, row: chooseSaleItem(e.row, id ? items.get(id) : undefined, null) }))} error={errFor('item')} aria-label="Item" />
      <TextInput label="Qty" size="xs" inputMode="decimal" classNames={{ input: 'lb-num' }} value={entry.row.quantity} aria-invalid={!!decimalTextError(entry.row.quantity)} onChange={(e) => { const q = e.currentTarget.value; set((x) => ({ ...x, row: setSaleQuantity(x.row, q) })); }} onFocus={(e) => e.currentTarget.select()} aria-label="Qty" />
      <TextInput label="Rate" size="xs" inputMode="decimal" classNames={{ input: 'lb-num' }} value={entry.row.rate} aria-invalid={!!decimalTextError(entry.row.rate)} onChange={(e) => { const r = e.currentTarget.value; set((x) => ({ ...x, row: setSaleRate(x.row, r) })); }} onFocus={(e) => e.currentTarget.select()} aria-label="Rate" />
      <MoneyField label="Amount" size="xs" blankZero value={entry.row.amount} onChange={(v) => set((x) => ({ ...x, row: setSaleAmount(x.row, v) }))} error={errFor('amount')} aria-label="Amount" />
      {useClasses && <ClassQuickFill label="Class" size="xs" value={entry.class_id} onChange={(class_id) => set({ class_id })} aria-label="Class" />}
      <DateField label="Due date" size="xs" value={entry.due_date} onChange={(due_date) => set({ due_date })} aria-label="Due date" />
      <TextInput label="Description" size="xs" value={entry.row.description} onChange={(e) => { const description = e.currentTarget.value; set((x) => ({ ...x, row: { ...x.row, description } })); }} aria-label="Description" className={mobile ? undefined : classes.chargeWide} />
      <Group gap="xs" align="flex-end" style={{ alignSelf: 'end' }}>
        <Button size="xs" onClick={() => void record()} loading={save.isPending} disabled={!customer || isBlankCharge(entry)}>
          Record
        </Button>
        <Button size="xs" variant="default" onClick={() => set(blankChargeEntry(entry.date, dueFor(entry.date)))} disabled={isBlankCharge(entry)}>
          Clear
        </Button>
      </Group>
    </div>
  );

  const list = !customer ? (
    <Text c="dimmed" size="sm" px={mobile ? 'sm' : 0}>
      Choose a customer or job to see its charges and payments, then enter a charge below.
    </Text>
  ) : txns.isLoading ? (
    <Skeleton h={200} />
  ) : txns.error ? (
    <Alert color="red">{describeError(txns.error)}</Alert>
  ) : mobile ? (
    <Stack gap={0}>
      {rows.length === 0 && (
        <Text c="dimmed" size="sm" px="sm">
          Nothing in this customer's register yet.
        </Text>
      )}
      {[...rows].reverse().map((r) => (
        <UnstyledButton key={r.summary.id} className={classes.mobileRow} onClick={() => open(paths.txn(r.summary.id))} data-testid="ar-row">
          <div style={{ minWidth: 0 }}>
            <Text size="sm" fw={600} truncate>
              {TXN_LABEL[r.summary.txn_type]} {r.summary.number ?? ''}
            </Text>
            <Text size="xs" c="dimmed" truncate>
              {formatDateShort(r.summary.date)}
              {r.summary.memo ? ` · ${r.summary.memo}` : ''}
            </Text>
          </div>
          <Text size="sm" className={classes.num} c={r.paid ? 'teal' : undefined}>
            {r.paid ? `-${formatMoney(r.paid)}` : formatMoney(r.charge)}
          </Text>
        </UnstyledButton>
      ))}
    </Stack>
  ) : (
    <div className={classes.list} role="grid" tabIndex={0} aria-label="Accounts Receivable register" aria-rowcount={rows.length} onKeyDown={onGridKey} data-testid="ar-register">
      <Table fz="sm" verticalSpacing={2} highlightOnHover stickyHeader>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Date</Table.Th>
            <Table.Th>Number</Table.Th>
            <Table.Th>Type</Table.Th>
            <Table.Th>Memo</Table.Th>
            <Table.Th ta="right">Amt Chrg</Table.Th>
            <Table.Th ta="right">Amt Paid</Table.Th>
            <Table.Th ta="right">Open</Table.Th>
            <Table.Th ta="right">Balance</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {rows.length === 0 && (
            <Table.Tr>
              <Table.Td colSpan={8}>
                <Text c="dimmed" size="sm">
                  Nothing in this customer's register yet.
                </Text>
              </Table.Td>
            </Table.Tr>
          )}
          {rows.map((r, i) => (
            <Table.Tr
              key={r.summary.id}
              className={classes.clickable}
              data-selected={i === selected || undefined}
              data-void={r.summary.is_void || undefined}
              aria-selected={i === selected}
              onClick={() => setSelected(i)}
              onDoubleClick={() => openRow(i)}
              data-testid="ar-row"
              style={i === selected ? { background: 'var(--mantine-primary-color-light)' } : undefined}
            >
              <Table.Td>{formatDateShort(r.summary.date)}</Table.Td>
              <Table.Td>{r.summary.number}</Table.Td>
              <Table.Td>{TXN_REGISTER_CODE[r.summary.txn_type]}</Table.Td>
              <Table.Td>
                <Text size="sm" truncate maw={320}>
                  {r.summary.is_void ? `VOID: ${r.summary.memo ?? ''}` : r.summary.memo}
                </Text>
              </Table.Td>
              <Table.Td ta="right" className={classes.num}>
                {formatMoney(r.charge, { blankZero: true })}
              </Table.Td>
              <Table.Td ta="right" className={classes.num}>
                {formatMoney(r.paid, { blankZero: true })}
              </Table.Td>
              <Table.Td ta="right" className={classes.num}>
                {r.summary.open_balance ? formatMoney(r.summary.open_balance) : ''}
              </Table.Td>
              <Table.Td ta="right" className={classes.num}>
                {formatMoney(r.balance)}
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
    </div>
  );

  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="statement-charges">
      {header}
      {list}
      {error?.field === 'form' && (
        <Alert color="red" role="alert" p="xs">
          {error.message}
        </Alert>
      )}
      <div style={mobile ? { padding: 'var(--mantine-spacing-sm)' } : undefined}>
        <Text size="xs" c="dimmed" mb={4}>
          New charge{customer ? '' : ' (choose a customer first)'}. Enter records it; a saved charge opens from its row.
        </Text>
        {entryFields}
      </div>
    </div>
  );
}
