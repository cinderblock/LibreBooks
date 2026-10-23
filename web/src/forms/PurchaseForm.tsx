// The purchase-side forms: Write Checks, Enter Bills, Enter Credit Card
// Charges, Enter Vendor Credits, Receive Items and Create Purchase Orders. One
// component, because they are one form in QBD too: a header saying who and when
// (and from which account), the Expenses and Items tabs, and the total. What
// differs per type is the header's face and which lines are allowed.

import { Alert, Button, Checkbox, Group, SegmentedControl, Select, Stack, Tabs, Text, TextInput, Textarea } from '@mantine/core';
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { useAccountIndex, useBalanceMap, useCompanyInfo, useEntityIndex, useItemIndex, useLoadOrder, useNextNumber, useOpenOrders, useTerms, useUnbilledReceipts } from '~/api/hooks';
import { describeError } from '~/api/errors';
import { notifications } from '@mantine/notifications';
import type { SaveOutcome, Txn, TxnSummary, TxnType } from '~/api/types';
import { withTrailingBlank } from '~/inputs/gridRows';
import { isOrderType, type OrderProgress } from '~/lib/orders';
import { stashTxn } from '~/windows/handoff';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { CreateFromOrderModal } from './CreateFromOrder';
import { drawRemaining, drawnLines, linesFromReceipt } from './linked';
import { DrawnFromNote, OrderProgressPanel } from './OrderProgress';
import { OrderPickerModal, ReceiptPickerModal } from './OrderPicker';
import { DateField } from '~/inputs/DateField';
import { LineGrid, type GridColumn } from '~/inputs/LineGrid';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, ClassQuickFill, ItemQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useDateWarning } from '~/inputs/useDateWarning';
import { CHECK_ACCOUNT_TYPES, naturalBalance } from '~/lib/accountTypes';
import { todayIso } from '~/lib/dates';
import { amountInWords, formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { paths } from '~/windows/paths';
import { gridErrors, gridHasErrors, type FormErrors, type TxnFormSpec } from './model';
import {
  applyPayee,
  applyTerms,
  hasCashAccount,
  isNonPosting,
  isPurchaseEmpty,
  newPurchaseDraft,
  nextPurchaseDraft,
  purchaseDraftFromTxn,
  purchaseDraftToTxn,
  purchaseFingerprint,
  purchaseServerErrors,
  purchaseTotal,
  setHeaderAmount,
  validatePurchase,
  vendorOnly,
  type PurchaseDraft,
  type PurchaseType,
} from './purchaseDraft';
import {
  blankExpense,
  blankItemRow,
  chooseItem,
  decimalTextError,
  expensesTotal,
  fromTxnLines,
  isBlankExpense,
  isBlankItem,
  itemsTotal,
  setItemAmount,
  setQuantity,
  setRate,
  type ExpenseRow,
  type ItemRow,
} from './purchaseLines';
import { FormBadges, TxnFormActions, useTxnForm } from './useTxnForm';
import classes from './forms.module.css';

/** The window/form slug for each purchase type (`/forms/<slug>/new`). */
export const PURCHASE_FORM_SLUG: Record<PurchaseType, string> = {
  Check: 'check',
  Bill: 'bill',
  CreditCardCharge: 'cc-charge',
  CreditCardCredit: 'cc-credit',
  VendorCredit: 'vendor-credit',
  ItemReceipt: 'receive-items',
  PurchaseOrder: 'purchase-order',
};

const NOUN: Record<PurchaseType, string> = {
  Check: 'check',
  Bill: 'bill',
  CreditCardCharge: 'credit card charge',
  CreditCardCredit: 'credit card credit',
  VendorCredit: 'vendor credit',
  ItemReceipt: 'item receipt',
  PurchaseOrder: 'purchase order',
};

const FORM_TITLE: Record<PurchaseType, string> = {
  Check: 'Write Checks',
  Bill: 'Enter Bills',
  CreditCardCharge: 'Enter Credit Card Charges',
  CreditCardCredit: 'Enter Credit Card Credits',
  VendorCredit: 'Enter Vendor Credits',
  ItemReceipt: 'Receive Items',
  PurchaseOrder: 'Create Purchase Orders',
};

const lastAccountKey = (t: PurchaseType) => `librebooks.purchase.${t}.account`;

function purchaseSpec(type: PurchaseType, defaultAccount: () => string | null): TxnFormSpec<PurchaseDraft> {
  return {
    noun: (d) => NOUN[d.txn_type],
    newDraft: () => newPurchaseDraft(type, todayIso(), defaultAccount()),
    fromTxn: purchaseDraftFromTxn,
    toTxn: purchaseDraftToTxn,
    validate: validatePurchase,
    isEmpty: isPurchaseEmpty,
    fingerprint: purchaseFingerprint,
    serverErrors: purchaseServerErrors,
    nextDraft: nextPurchaseDraft,
    newPath: type === 'Check' ? paths.newCheck() : paths.form(PURCHASE_FORM_SLUG[type]),
    savedMessage: (out: SaveOutcome) => `${capitalize(NOUN[out.txn.txn_type as PurchaseType] ?? 'transaction')} ${out.txn.number ?? ''} for ${formatMoney(out.total, { symbol: true })} saved.`.replace('  ', ' '),
    siblings: (d) => ({ types: [d.txn_type], ...(hasCashAccount(d.txn_type) && d.account_id ? { account_id: d.account_id } : {}) }),
    title: (d, saved) => {
      const base = saved ? `${TXN_LABEL[d.txn_type]}${saved.number ? ` ${saved.number}` : ''}` : FORM_TITLE[d.txn_type];
      return base;
    },
  };
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export function PurchaseForm({ type, saved, initial }: { type: PurchaseType; saved: Txn | null; initial: Txn | null }) {
  const api = useApi();
  const co = useCompanyId();
  const { data: info } = useCompanyInfo();
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const items = useItemIndex();
  const balances = useBalanceMap();
  const { data: terms = [] } = useTerms();
  const firstField = useRef<HTMLInputElement>(null);
  const open = useOpenWindow();
  /** Receive Items / Enter Bills: the vendor's open purchase orders to draw on (Select PO). */
  const [pickPo, setPickPo] = useState(false);
  /** Enter Bills: the vendor's item receipts not yet billed (Enter Bill for Received Items). */
  const [pickReceipt, setPickReceipt] = useState(false);
  const loadOrder = useLoadOrder();
  /** Create Item Receipt / Enter Bill from a saved purchase order. */
  const [drawing, setDrawing] = useState<{ order: Txn; progress: OrderProgress; target: TxnType } | null>(null);

  const defaultAccount = useCallback(() => {
    const remembered = localStorage.getItem(lastAccountKey(type));
    if (remembered) return remembered;
    if (type === 'Check') return info?.preferences.default_check_account ?? null;
    return null;
  }, [type, info]);
  const spec = useMemo(() => purchaseSpec(type, defaultAccount), [type, defaultAccount]);
  const form = useTxnForm({ spec, saved, initial, onSavedNew: () => firstField.current?.focus() });
  const { draft, update, errors, mobile, enterBehavior } = form;
  const t = draft.txn_type;
  const cash = hasCashAccount(t);

  // The company's default account arrives after first render on a cold load.
  useEffect(() => {
    if (cash && !draft.account_id && draft.edit_seq === 0 && !form.dirty) {
      const d = defaultAccount();
      if (d) update({ account_id: d });
    }
  }, [info]); // eslint-disable-line react-hooks/exhaustive-deps

  // Next number (checks per account; orders per type), until the user types one.
  const wantsAuto = draft.edit_seq === 0 && draft.numberIsAuto && (t === 'Check' || t === 'PurchaseOrder');
  const next = useNextNumber(t, draft.account_id, wantsAuto);
  useEffect(() => {
    if (next.data !== undefined && wantsAuto) update((d) => (d.numberIsAuto ? { ...d, number: next.data ?? '' } : d));
  }, [next.data]); // eslint-disable-line react-hooks/exhaustive-deps

  // Remember the account a saved check/charge used, as the default next time.
  useEffect(() => {
    if (cash && draft.account_id && form.saved) localStorage.setItem(lastAccountKey(t), draft.account_id);
  }, [form.saved?.edit_seq]); // eslint-disable-line react-hooks/exhaustive-deps

  const account = draft.account_id ? accounts.get(draft.account_id) : undefined;
  const dateWarning = useDateWarning(draft.date);
  const payee = draft.entity_id ? entities.get(draft.entity_id) : undefined;
  const useClasses = info?.preferences.use_class_tracking ?? false;
  const total = purchaseTotal(draft);
  const receives = t === 'ItemReceipt' || t === 'Bill';
  const openPos = useOpenOrders('PurchaseOrder', draft.entity_id, receives);
  const openPoCount = openPos.data?.length ?? 0;
  const unbilled = useUnbilledReceipts(draft.entity_id, t === 'Bill' && !form.saved);
  const unbilledCount = unbilled.data?.length ?? 0;

  /** Lines drawn from orders or receipts join the form: the vendor's prefilled expense line (its default account, no amount) gives way. */
  const addLines = (lines: Txn['lines']) => {
    if (!lines.length) return;
    const { items: rows, expenses } = fromTxnLines(lines);
    update((d) => ({
      ...d,
      expenses: withTrailingBlank([...d.expenses.filter((r) => !(r.account_id && r.amount === null && !r.memo.trim() && !r.entity_id)), ...expenses.map((r) => ({ ...r, lineId: null }))], isBlankExpense, blankExpense),
      items: withTrailingBlank([...d.items.filter((r) => !isBlankItem(r)), ...rows.map((r) => ({ ...r, lineId: null }))], isBlankItem, blankItemRow),
    }));
  };
  /** Select PO: the chosen orders' remaining lines join the Items tab, each linked to its PO line (quantities editable for a partial receipt). */
  const addFromOrders = async (picked: TxnSummary[]) => {
    setPickPo(false);
    try {
      const orders = await Promise.all(picked.map((o) => loadOrder(o.id)));
      addLines(orders.flatMap((o) => drawnLines(o.order, o.progress, drawRemaining(o.progress))));
    } catch (e) {
      notifications.show({ message: describeError(e), color: 'red' });
    }
  };
  /** Select receipt: the chosen receipts' lines join the bill whole, each linked to its receipt line (QBD's Enter Bill for Received Items). */
  const addFromReceipts = async (picked: TxnSummary[]) => {
    setPickReceipt(false);
    try {
      const receipts = await Promise.all(picked.map((r) => api.txn(co, r.id)));
      addLines(receipts.flatMap((r) => linesFromReceipt(r.txn)));
    } catch (e) {
      notifications.show({ message: describeError(e), color: 'red' });
    }
  };
  /** A receipt or bill drawn from a saved PO opens in its own window. */
  const onDrawn = (doc: Txn) => {
    setDrawing(null);
    open(paths.form(doc.txn_type === 'Bill' ? 'bill' : 'receive-items', stashTxn(doc)));
  };

  const onPayee = async (id: string | null) => {
    const who = id ? entities.get(id) : undefined;
    let last: Txn | null = null;
    if (who && draft.edit_seq === 0 && total === 0) {
      try {
        const [summary] = await api.txns(co, { types: [t], entity_id: who.id, descending: true, limit: 1 });
        if (summary) last = (await api.txn(co, summary.id)).txn;
      } catch {
        // Recall is a convenience; without it the default account still applies.
      }
    }
    update((d) => applyPayee(d, who, last, who?.terms_id ? terms.find((x) => x.id === who.terms_id) : null));
  };

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
    if (t === 'PurchaseOrder') {
      // QBD's "Clsd": the line is closed by hand, nothing more expected against it (`TxnLine.is_closed`).
      cols.push({
        key: 'closed',
        header: 'Clsd',
        width: 48,
        align: 'center',
        mobile: 'half',
        render: (c) => (
          <Checkbox
            size="xs"
            label={c.variant === 'default' ? 'Closed' : undefined}
            styles={{ root: { display: 'flex', justifyContent: 'center' } }}
            checked={c.row.is_closed}
            onChange={(e) => {
              const is_closed = e.currentTarget.checked;
              c.update((r) => ({ ...r, is_closed }));
            }}
            {...c.cellProps}
          />
        ),
      });
    }
    return cols;
  }, [draft.items, items, info, t]); // eslint-disable-line react-hooks/exhaustive-deps

  const onEnterSave = useCallback(() => void form.save('close'), []); // eslint-disable-line react-hooks/exhaustive-deps

  const accountBalance = account ? naturalBalance(account.account_type, balances.get(account.id) ?? 0) : null;
  const address = payee
    ? [payee.print_as ?? payee.company_name ?? payee.name, ...(payee.bill_address.lines ?? []), [payee.bill_address.city, payee.bill_address.state, payee.bill_address.postal_code].filter(Boolean).join(' ')]
        .filter(Boolean)
        .join('\n')
    : '';

  const numberField = (label: string, inputMode: 'numeric' | 'text' = 'numeric', aria = label) => (
    <TextInput
      label={label}
      value={draft.to_be_printed && t === 'Check' ? 'To Print' : draft.number}
      disabled={draft.to_be_printed && t === 'Check'}
      onChange={(e) => {
        const number = e.currentTarget.value;
        update((d) => ({ ...d, number, numberIsAuto: false }));
      }}
      error={errors.header.number}
      inputMode={inputMode}
      aria-label={aria}
    />
  );
  const dateField = <DateField label="Date" value={draft.date} onChange={(date) => date && update((d) => (d.terms_id ? applyTerms({ ...d, date }, terms.find((x) => x.id === d.terms_id)) : { ...d, date }))} error={errors.header.date} warning={dateWarning} aria-label="Date" />;
  const memoField = (
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
  );
  const amountField = (label: string, aria = label) => (
    <MoneyField label={label} value={total} onChange={(v) => update((d) => setHeaderAmount(d, v))} error={errors.header.amount} aria-label={aria} size={mobile ? 'md' : 'sm'} />
  );
  const nameField = (label: string, aria = label) => (
    <NameQuickFill
      label={label}
      kinds={vendorOnly(t) ? ['Vendor'] : undefined}
      value={draft.entity_id}
      onChange={(id) => void onPayee(id)}
      error={errors.header.entity}
      inputRef={firstField}
      autoFocus={!form.saved && !mobile}
      aria-label={aria}
    />
  );

  // QBD asks "open purchase orders exist for this vendor" on a receipt or bill; Select PO brings their lines in.
  // On a bill, Select receipt bills the items a receipt brought in (Enter Bill for Received Items).
  const selectPo = receives && (
    <Group gap="xs" wrap="wrap" align="center">
      <Button size="xs" variant="default" onClick={() => setPickPo(true)} disabled={!draft.entity_id}>
        Select PO
      </Button>
      {openPoCount > 0 && (
        <Alert color="blue" p={4} px="xs" data-testid="open-purchase-orders">
          <Text size="sm">
            {openPoCount} open purchase {openPoCount === 1 ? 'order' : 'orders'} for this vendor.
          </Text>
        </Alert>
      )}
      {t === 'Bill' && !form.saved && (
        <Button size="xs" variant="default" onClick={() => setPickReceipt(true)} disabled={!draft.entity_id}>
          Select receipt
        </Button>
      )}
      {unbilledCount > 0 && (
        <Alert color="blue" p={4} px="xs" data-testid="unbilled-receipts">
          <Text size="sm">
            {unbilledCount} item {unbilledCount === 1 ? 'receipt' : 'receipts'} not yet billed.
          </Text>
        </Alert>
      )}
    </Group>
  );

  let header: ReactNode;
  if (t === 'Check') {
    header = (
      <>
        <Group align="flex-end" gap="md" wrap={mobile ? 'wrap' : 'nowrap'}>
          <AccountQuickFill
            label="Bank Account"
            types={CHECK_ACCOUNT_TYPES}
            value={draft.account_id}
            onChange={(id) => update((d) => ({ ...d, account_id: id, numberIsAuto: d.edit_seq === 0 ? true : d.numberIsAuto }))}
            error={errors.header.account}
            style={{ flex: mobile ? '1 1 100%' : '0 1 320px' }}
            aria-label="Bank account"
          />
          <Text size="sm" c="dimmed" pb={6}>
            Ending Balance{' '}
            <Text span fw={600} c="var(--mantine-color-text)">
              {accountBalance === null ? '—' : formatMoney(accountBalance)}
            </Text>
          </Text>
          <FormBadges saved={form.saved} />
        </Group>
        <div className={mobile ? undefined : classes.checkFace}>
          <Stack gap="sm" className={mobile ? undefined : classes.wide} style={mobile ? undefined : { gridColumn: '1 / 2' }}>
            {nameField('Pay to the Order of', 'Payee')}
          </Stack>
          {numberField('No.', 'numeric', 'Check number')}
          {dateField}
          {amountField('$ Amount', 'Check amount')}
          {!mobile && (
            <div className={classes.full}>
              <div className={classes.words} aria-label="Amount in words">
                {total > 0 ? `${amountInWords(total)} ${'*'.repeat(Math.max(0, 60 - amountInWords(total).length))}` : ''}
              </div>
            </div>
          )}
          {!mobile && <Textarea label="Address" value={address} readOnly autosize minRows={3} maxRows={5} tabIndex={-1} className={classes.wide} />}
          {memoField}
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
  } else if (t === 'CreditCardCharge' || t === 'CreditCardCredit') {
    header = (
      <>
        <Group align="flex-end" gap="md" wrap="wrap">
          <AccountQuickFill
            label="Credit Card"
            types={['CreditCard']}
            value={draft.account_id}
            onChange={(id) => update({ account_id: id })}
            error={errors.header.account}
            style={{ flex: mobile ? '1 1 100%' : '0 1 320px' }}
            aria-label="Credit card account"
          />
          <SegmentedControl
            data={[
              { value: 'CreditCardCharge', label: 'Purchase/Charge' },
              { value: 'CreditCardCredit', label: 'Refund/Credit' },
            ]}
            value={t}
            onChange={(v) => update({ txn_type: v as PurchaseType })}
            disabled={!!form.saved}
          />
          <Text size="sm" c="dimmed" pb={6}>
            Ending Balance{' '}
            <Text span fw={600} c="var(--mantine-color-text)">
              {accountBalance === null ? '—' : formatMoney(accountBalance)}
            </Text>
          </Text>
          <FormBadges saved={form.saved} />
        </Group>
        <div className={mobile ? undefined : classes.headerGrid}>
          {nameField('Purchased From')}
          {dateField}
          {numberField('Ref No.', 'text')}
          {amountField('Amount')}
          {memoField}
        </div>
      </>
    );
  } else if (t === 'Bill' || t === 'VendorCredit') {
    header = (
      <>
        <Group align="flex-end" gap="md" wrap="wrap">
          <SegmentedControl
            data={[
              { value: 'Bill', label: 'Bill' },
              { value: 'VendorCredit', label: 'Credit' },
            ]}
            value={t}
            onChange={(v) => update({ txn_type: v as PurchaseType })}
            disabled={!!form.saved}
          />
          <FormBadges saved={form.saved} />
          <DrawnFromNote txn={draft.base ?? form.saved} />
          {selectPo}
        </Group>
        <div className={mobile ? undefined : classes.headerGrid}>
          {nameField('Vendor')}
          {dateField}
          {numberField('Ref. No.', 'text')}
          {amountField(t === 'Bill' ? 'Amount Due' : 'Credit Amount')}
          {t === 'Bill' && (
            <Select
              label="Terms"
              data={terms.filter((x) => x.is_active || x.id === draft.terms_id).map((x) => ({ value: x.id, label: x.name }))}
              value={draft.terms_id}
              onChange={(id) => update((d) => applyTerms(d, terms.find((x) => x.id === id) ?? null))}
              clearable
              searchable
              aria-label="Terms"
            />
          )}
          {t === 'Bill' && <DateField label="Bill Due" value={draft.due_date} onChange={(due_date) => update({ due_date })} aria-label="Bill due" />}
          {!mobile && <Textarea label="Address" value={address} readOnly autosize minRows={2} maxRows={5} tabIndex={-1} className={classes.wide} />}
          {memoField}
        </div>
      </>
    );
  } else {
    // Item receipts and purchase orders.
    header = (
      <>
        <Group align="flex-end" gap="md" wrap="wrap">
          <FormBadges saved={form.saved} nonPosting={isNonPosting(t)} />
          {isNonPosting(t) && (
            <Text size="sm" c="dimmed">
              A purchase order is a record of what was ordered. It posts nothing; the bill or check for it does.
            </Text>
          )}
          <DrawnFromNote txn={draft.base ?? form.saved} />
          {selectPo}
        </Group>
        <div className={mobile ? undefined : classes.headerGrid}>
          {nameField('Vendor')}
          {dateField}
          {numberField(t === 'PurchaseOrder' ? 'P.O. No.' : 'Ref. No.', 'text')}
          {amountField('Total')}
          {!mobile && <Textarea label="Vendor address" value={address} readOnly autosize minRows={2} maxRows={5} tabIndex={-1} className={classes.wide} />}
          {memoField}
        </div>
      </>
    );
  }

  const itemsOnly = t === 'PurchaseOrder';
  const itemsGrid = (
    <LineGrid
      label="Item lines"
      rows={draft.items}
      onChange={(rows) => update({ items: rows })}
      columns={itemColumns}
      rowKey={(r) => r.key}
      isBlank={isBlankItem}
      makeBlank={blankItemRow}
      errors={gridErrors(errors, 'items')}
      enterBehavior={enterBehavior}
      onEnterSave={onEnterSave}
      mobile={mobile}
      minVisibleRows={5}
    />
  );

  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid={t === 'Check' ? 'write-checks' : `form-${PURCHASE_FORM_SLUG[t]}`}>
      {form.banner}
      {header}
      {form.formErrors}
      {itemsOnly ? (
        itemsGrid
      ) : (
        <PurchaseTabs
          errors={errors}
          initial={form.saved && form.saved.lines.length > 0 && form.saved.lines.every((l) => l.kind === 'Item') ? 'items' : t === 'ItemReceipt' ? 'items' : 'expenses'}
          expensesTotal={expensesTotal(draft.expenses)}
          itemsTotal={itemsTotal(draft.items)}
          expenses={
            <LineGrid
              label="Expense lines"
              rows={draft.expenses}
              onChange={(expenses) => update({ expenses })}
              columns={expenseColumns}
              rowKey={(r) => r.key}
              isBlank={isBlankExpense}
              makeBlank={blankExpense}
              errors={gridErrors(errors, 'expenses')}
              enterBehavior={enterBehavior}
              onEnterSave={onEnterSave}
              mobile={mobile}
              minVisibleRows={5}
            />
          }
          items={itemsGrid}
        />
      )}
      {form.saved && isOrderType(form.saved.txn_type) && <OrderProgressPanel order={form.saved} dirty={form.dirty} onDraw={(target, progress) => setDrawing({ order: form.saved!, progress, target })} />}
      <TxnFormActions form={form} hideVoid={isNonPosting(t)} />
      {form.chrome}
      <CreateFromOrderModal order={drawing?.order ?? null} progress={drawing?.progress ?? null} target={drawing?.target ?? 'ItemReceipt'} onClose={() => setDrawing(null)} onCreate={onDrawn} />
      <OrderPickerModal opened={pickPo} type="PurchaseOrder" entityId={draft.entity_id} multiple onClose={() => setPickPo(false)} onPick={(o) => void addFromOrders(o)} />
      <ReceiptPickerModal opened={pickReceipt} entityId={draft.entity_id} onClose={() => setPickReceipt(false)} onPick={(r) => void addFromReceipts(r)} />
    </div>
  );
}

/** The Expenses / Items tabs with their totals; errors colour the tab that has them. */
function PurchaseTabs({ errors, initial, expensesTotal: et, itemsTotal: it, expenses, items }: { errors: FormErrors; initial: 'expenses' | 'items'; expensesTotal: number; itemsTotal: number; expenses: ReactNode; items: ReactNode }) {
  return (
    <Tabs defaultValue={initial} keepMounted>
      <Tabs.List>
        <Tabs.Tab value="expenses" color={gridHasErrors(errors, 'expenses') ? 'red' : undefined}>
          Expenses <Text span fw={600}>{formatMoney(et)}</Text>
        </Tabs.Tab>
        <Tabs.Tab value="items" color={gridHasErrors(errors, 'items') ? 'red' : undefined}>
          Items <Text span fw={600}>{formatMoney(it)}</Text>
        </Tabs.Tab>
      </Tabs.List>
      <Tabs.Panel value="expenses" pt="xs">
        {expenses}
      </Tabs.Panel>
      <Tabs.Panel value="items" pt="xs">
        {items}
      </Tabs.Panel>
    </Tabs>
  );
}
