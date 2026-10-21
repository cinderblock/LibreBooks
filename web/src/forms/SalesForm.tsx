// The sales-side forms: Create Invoices, Enter Sales Receipts, Create Credit
// Memos/Refunds and Create Estimates. One component: a customer header with
// terms, addresses and sales tax over item lines, totals that the server's
// preview confirms (tax is the server's call), and a "postings" panel showing
// what saving will do.

import { Alert, Button, Checkbox, Collapse, Group, Select, Table, Text, TextInput, Textarea } from '@mantine/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useAccountIndex, useCompanyInfo, useEntityIndex, useIndex, useItemIndex, useItems, useNextNumber, useOpenOrders, usePaymentMethods, useSalesReps, useSalesTaxCodes, useTerms, useTxnPreview, useTxns } from '~/api/hooks';
import type { Address, SalesTaxCode, SaveOutcome, Txn } from '~/api/types';
import { warningText } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { LineGrid, type GridColumn } from '~/inputs/LineGrid';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, ClassQuickFill, ItemQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useDateWarning } from '~/inputs/useDateWarning';
import { todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { isOrderType, type OrderProgress } from '~/lib/orders';
import { TXN_LABEL } from '~/lib/txnTypes';
import { stashTxn } from '~/windows/handoff';
import { paths } from '~/windows/paths';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { CreateFromOrderModal } from './CreateFromOrder';
import { gridErrors, type TxnFormSpec } from './model';
import { DrawnFromNote, OrderProgressPanel } from './OrderProgress';
import { OrderPickerModal } from './OrderPicker';
import {
  applyCustomer,
  applySalesTerms,
  hasDueDate,
  isNonPostingSale,
  isSalesEmpty,
  newSalesDraft,
  nextSalesDraft,
  salesDraftFromTxn,
  salesDraftToTxn,
  salesFingerprint,
  salesServerErrors,
  salesTotals,
  setSaleLines,
  validateSales,
  type SalesDefaults,
  type SalesDraft,
  type SalesType,
} from './salesDraft';
import { blankSaleRow, chooseSaleItem, isBlankSale, setSaleAmount, setSaleQuantity, setSaleRate, taxRate, type SaleRow } from './salesLines';
import { decimalTextError } from './purchaseLines';
import { FormBadges, TxnFormActions, useTxnForm } from './useTxnForm';
import classes from './forms.module.css';

export const SALES_FORM_SLUG: Record<SalesType, string> = {
  Invoice: 'invoice',
  SalesReceipt: 'sales-receipt',
  CreditMemo: 'credit-memo',
  Estimate: 'estimate',
  SalesOrder: 'sales-order',
  StatementCharge: 'statement-charge',
};

const NOUN: Record<SalesType, string> = { Invoice: 'invoice', SalesReceipt: 'sales receipt', CreditMemo: 'credit memo', Estimate: 'estimate', SalesOrder: 'sales order', StatementCharge: 'statement charge' };
const FORM_TITLE: Record<SalesType, string> = {
  Invoice: 'Create Invoices',
  SalesReceipt: 'Enter Sales Receipts',
  CreditMemo: 'Create Credit Memos/Refunds',
  Estimate: 'Create Estimates',
  SalesOrder: 'Create Sales Orders',
  StatementCharge: 'Statement Charge',
};
const NUMBER_LABEL: Record<SalesType, string> = { Invoice: 'Invoice #', SalesReceipt: 'Sale No.', CreditMemo: 'Credit No.', Estimate: 'Estimate #', SalesOrder: 'S.O. No.', StatementCharge: 'Number' };
const NON_POSTING_NOTE: Partial<Record<SalesType, string>> = {
  Estimate: 'An estimate is a quote. It posts nothing; the invoice made from it does.',
  SalesOrder: 'A sales order is a promise to sell. It posts nothing; the invoice made from it does.',
};
const TAX_ITEM_KEY = 'librebooks.sales.taxItem';

/** Address ↔ the textarea people edit (one line per row; the structured fields stay). */
function addressText(a: Address): string {
  return [...(a.lines ?? []), [a.city, a.state, a.postal_code].filter(Boolean).join(' ')].filter(Boolean).join('\n');
}
function parseAddress(text: string, prev: Address): Address {
  const lines = text.split('\n').map((l) => l.trim()).filter(Boolean);
  // Keep the structured city/state/zip when the last line still reads like them.
  const last = lines[lines.length - 1];
  const structured = [prev.city, prev.state, prev.postal_code].filter(Boolean).join(' ');
  if (last && structured && last === structured) return { ...prev, lines: lines.slice(0, -1) };
  return { ...prev, lines, city: null, state: null, postal_code: null };
}

export function SalesForm({ type, saved, initial }: { type: SalesType; saved: Txn | null; initial: Txn | null }) {
  const { data: info } = useCompanyInfo();
  const prefs = info?.preferences;
  const entities = useEntityIndex();
  const items = useItemIndex();
  const accounts = useAccountIndex();
  const { data: itemList = [] } = useItems();
  const { data: codeList = [] } = useSalesTaxCodes();
  const codes = useIndex(codeList) as Map<string, SalesTaxCode>;
  const { data: terms = [] } = useTerms();
  const { data: reps = [] } = useSalesReps();
  const { data: methods = [] } = usePaymentMethods();
  const firstField = useRef<HTMLInputElement>(null);
  const open = useOpenWindow();
  const [showPostings, setShowPostings] = useState(false);
  /** The order being drawn on: Create Invoice on a saved order, or a sales order picked for a new invoice. */
  const [drawing, setDrawing] = useState<{ order: Txn; progress: OrderProgress } | null>(null);
  const [pickOrder, setPickOrder] = useState(false);

  const taxItems = useMemo(() => itemList.filter((i) => (i.item_type === 'SalesTax' || i.item_type === 'SalesTaxGroup') && i.is_active), [itemList]);
  const defaults = useMemo<SalesDefaults>(
    () => ({
      taxItemId: prefs?.sales_tax_enabled === false ? null : (localStorage.getItem(TAX_ITEM_KEY) ?? taxItems[0]?.id ?? null),
      taxCodeId: codeList.find((c) => c.is_taxable && c.name === 'Tax')?.id ?? codeList.find((c) => c.is_taxable)?.id ?? null,
      depositTo: prefs?.use_undeposited_funds === false ? (prefs.default_deposit_account ?? null) : null,
    }),
    [prefs, taxItems, codeList],
  );

  const spec = useMemo<TxnFormSpec<SalesDraft>>(
    () => ({
      noun: (d) => NOUN[d.txn_type],
      newDraft: () => newSalesDraft(type, todayIso(), defaults),
      fromTxn: salesDraftFromTxn,
      toTxn: (d) => salesDraftToTxn(d, items, codes),
      validate: validateSales,
      isEmpty: isSalesEmpty,
      fingerprint: salesFingerprint,
      serverErrors: salesServerErrors,
      nextDraft: (d) => nextSalesDraft(d, defaults),
      newPath: paths.form(SALES_FORM_SLUG[type]),
      savedMessage: (out: SaveOutcome) => `${capitalize(NOUN[(out.txn.txn_type as SalesType) ?? type] ?? 'transaction')} ${out.txn.number ?? ''} for ${formatMoney(out.total, { symbol: true })} saved.`.replace('  ', ' '),
      siblings: (d) => ({ types: [d.txn_type] }),
      title: (d, s) => (s ? `${TXN_LABEL[d.txn_type]}${s.number ? ` ${s.number}` : ''}` : FORM_TITLE[d.txn_type]),
    }),
    [type, defaults, items, codes],
  );
  const form = useTxnForm({ spec, saved, initial, onSavedNew: () => firstField.current?.focus() });
  const { draft, update, errors, mobile, enterBehavior } = form;
  const t = draft.txn_type;

  // Defaults (tax item, tax code) arrive after the lists load on a cold start.
  useEffect(() => {
    if (draft.edit_seq === 0 && !form.dirty) {
      if (!draft.sales_tax_item_id && defaults.taxItemId) update({ sales_tax_item_id: defaults.taxItemId });
      if (!draft.customer_tax_code_id && defaults.taxCodeId) update({ customer_tax_code_id: defaults.taxCodeId });
    }
  }, [defaults]); // eslint-disable-line react-hooks/exhaustive-deps

  const wantsAuto = draft.edit_seq === 0 && draft.numberIsAuto;
  const next = useNextNumber(t, null, wantsAuto);
  useEffect(() => {
    if (next.data !== undefined && wantsAuto) update((d) => (d.numberIsAuto ? { ...d, number: next.data ?? '' } : d));
  }, [next.data, wantsAuto]); // eslint-disable-line react-hooks/exhaustive-deps

  // Lines change → derived amounts (subtotals, percent discounts) follow.
  const setLines = (rows: SaleRow[]) => update((d) => setSaleLines(d, rows, items));

  const onCustomer = (id: string | null) => {
    const c = id ? entities.get(id) : undefined;
    const parent = c?.parent_id ? entities.get(c.parent_id) : undefined;
    const termsId = c?.terms_id ?? parent?.terms_id ?? null;
    update((d) => applyCustomer(d, c, termsId ? terms.find((x) => x.id === termsId) : null, parent));
  };

  const totals = salesTotals(draft, items, codes);
  const txnForPreview = useMemo(() => (form.dirty || saved ? salesDraftToTxn(draft, items, codes) : null), [draft, items, codes, form.dirty, saved]);
  const preview = useTxnPreview(txnForPreview, !isNonPostingSale(t));
  const serverTotals = preview.data && !preview.isPlaceholderData ? preview.data : null;
  const tax = serverTotals ? serverTotals.sales_tax : totals.tax;
  const total = serverTotals ? serverTotals.total : totals.total;
  const taxItem = draft.sales_tax_item_id ? items.get(draft.sales_tax_item_id) : undefined;

  // A saved invoice's balance due, from the transaction list (open_balance).
  const mine = useTxns({ types: [t], entity_id: draft.entity_id ?? undefined, include_jobs: true, include_void: true, limit: 200 }, !!saved && hasDueDate(t));
  const openBalance = saved ? (mine.data?.find((s) => s.id === saved.id)?.open_balance ?? null) : null;

  const dateWarning = useDateWarning(draft.date);
  const useClasses = prefs?.use_class_tracking ?? false;
  const salesTax = prefs?.sales_tax_enabled ?? true;
  // A new invoice for a customer with open sales orders may draw on one (QBD's Available Sales Orders).
  const openOrders = useOpenOrders('SalesOrder', draft.entity_id, t === 'Invoice' && !saved && prefs?.sales_orders_enabled !== false);
  const openOrderCount = openOrders.data?.length ?? 0;

  const columns = useMemo<GridColumn<SaleRow>[]>(() => {
    const cols: GridColumn<SaleRow>[] = [
      {
        key: 'item',
        header: 'Item',
        width: '18%',
        render: (c) => (
          <ItemQuickFill
            side="sell"
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            value={c.row.item_id}
            onChange={(id) => c.update((r) => chooseSaleItem(r, id ? items.get(id) : undefined, defaults.taxCodeId))}
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
        width: 64,
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
            aria-invalid={!!decimalTextError(c.row.quantity)}
            onChange={(e) => {
              const q = e.currentTarget.value;
              c.update((r) => setSaleQuantity(r, q));
            }}
            onFocus={(e) => e.currentTarget.select()}
            {...c.cellProps}
          />
        ),
      },
      {
        key: 'rate',
        header: 'Rate',
        width: 96,
        align: 'right',
        mobile: 'half',
        render: (c) => (
          <TextInput
            size="xs"
            variant={c.variant === 'cell' ? 'unstyled' : 'default'}
            label={c.variant === 'default' ? (c.row.rate_is_percent ? 'Percent' : c.label) : undefined}
            classNames={c.variant === 'cell' ? { input: 'lb-cell-text lb-num' } : { input: 'lb-num' }}
            inputMode="decimal"
            value={c.row.rate}
            rightSection={c.row.rate_is_percent ? <Text size="xs">%</Text> : undefined}
            aria-invalid={!!decimalTextError(c.row.rate)}
            onChange={(e) => {
              const rate = e.currentTarget.value;
              c.update((r) => setSaleRate(r, rate));
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
            allowNegative
            value={c.row.amount}
            onChange={(v) => c.update((r) => setSaleAmount(r, v))}
            error={c.error}
            inputProps={c.cellProps}
          />
        ),
        footer: <span className={classes.total}>{formatMoney(totals.subtotal)}</span>,
      },
    ];
    if (salesTax) {
      cols.push({
        key: 'tax',
        header: 'Tax',
        width: 72,
        mobile: 'half',
        render: (c) => (
          <Select
            size="xs"
            variant={c.variant === 'cell' ? 'unstyled' : 'default'}
            label={c.variant === 'default' ? 'Tax code' : undefined}
            classNames={c.variant === 'cell' ? { input: 'lb-cell-text' } : undefined}
            data={codeList.filter((x) => x.is_active || x.id === c.row.sales_tax_code_id).map((x) => ({ value: x.id, label: x.name }))}
            value={c.row.sales_tax_code_id}
            onChange={(sales_tax_code_id) => c.update((r) => ({ ...r, sales_tax_code_id }))}
            allowDeselect={false}
            {...c.cellProps}
          />
        ),
      });
    }
    if (useClasses) {
      cols.push({
        key: 'class',
        header: 'Class',
        width: '12%',
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
  }, [items, codeList, defaults.taxCodeId, useClasses, salesTax, totals.subtotal]); // eslint-disable-line react-hooks/exhaustive-deps

  const onEnterSave = useCallback(() => void form.save('close'), []); // eslint-disable-line react-hooks/exhaustive-deps

  const invoice = t === 'Invoice';
  const receipt = t === 'SalesReceipt';
  const charge = t === 'StatementCharge';
  const dated = hasDueDate(t);
  const shipped = invoice || t === 'Estimate' || t === 'SalesOrder';
  const nonPosting = isNonPostingSale(t);

  /** A drawn document: from a saved order it opens in its own window; on a new invoice it fills this form. */
  const onDrawn = (doc: Txn) => {
    setDrawing(null);
    if (saved) open(paths.form('invoice', stashTxn(doc)));
    else form.reset(salesDraftFromTxn(doc));
  };

  const header = (
    <>
      <Group align="flex-end" gap="md" wrap="wrap">
        <FormBadges saved={form.saved} nonPosting={nonPosting} />
        {NON_POSTING_NOTE[t] && (
          <Text size="sm" c="dimmed">
            {NON_POSTING_NOTE[t]}
          </Text>
        )}
        <DrawnFromNote txn={draft.base ?? saved} />
        {openOrderCount > 0 && (
          <Alert color="blue" p="xs" data-testid="available-sales-orders">
            <Group gap="xs" wrap="wrap">
              <Text size="sm">
                This customer has {openOrderCount} open sales {openOrderCount === 1 ? 'order' : 'orders'}.
              </Text>
              <Button size="xs" variant="light" onClick={() => setPickOrder(true)}>
                Invoice from a sales order…
              </Button>
            </Group>
          </Alert>
        )}
        {openBalance !== null && (
          <Text size="sm" c="dimmed">
            Balance due{' '}
            <Text span fw={600} c="var(--mantine-color-text)">
              {formatMoney(openBalance)}
            </Text>
          </Text>
        )}
      </Group>
      <div className={mobile ? undefined : classes.headerGrid}>
        <NameQuickFill
          label="Customer:Job"
          kinds={['Customer']}
          value={draft.entity_id}
          onChange={onCustomer}
          error={errors.header.entity}
          inputRef={firstField}
          autoFocus={!form.saved && !mobile}
          aria-label="Customer:Job"
        />
        {useClasses ? <ClassQuickFill label="Class" value={draft.class_id} onChange={(class_id) => update({ class_id })} aria-label="Class" /> : <span />}
        <DateField
          label="Date"
          value={draft.date}
          onChange={(date) => date && update((d) => (dated && d.terms_id ? applySalesTerms({ ...d, date }, terms.find((x) => x.id === d.terms_id)) : { ...d, date }))}
          error={errors.header.date}
          warning={dateWarning}
          aria-label="Date"
        />
        <TextInput
          label={NUMBER_LABEL[t]}
          value={draft.number}
          onChange={(e) => {
            const number = e.currentTarget.value;
            update((d) => ({ ...d, number, numberIsAuto: false }));
          }}
          error={errors.header.number}
          aria-label={NUMBER_LABEL[t]}
        />
        {!mobile && !charge && (
          <Textarea
            label="Bill To"
            value={addressText(draft.bill_address)}
            onChange={(e) => {
              const text = e.currentTarget.value;
              update((d) => ({ ...d, bill_address: parseAddress(text, d.bill_address) }));
            }}
            autosize
            minRows={3}
            maxRows={5}
            className={classes.wide}
          />
        )}
        {!mobile && shipped && (
          <Textarea
            label="Ship To"
            value={addressText(draft.ship_address)}
            onChange={(e) => {
              const text = e.currentTarget.value;
              update((d) => ({ ...d, ship_address: parseAddress(text, d.ship_address) }));
            }}
            autosize
            minRows={3}
            maxRows={5}
            className={classes.wide}
          />
        )}
        {shipped && (
          <TextInput
            label="P.O. No."
            value={draft.po_number}
            onChange={(e) => {
              const po_number = e.currentTarget.value;
              update({ po_number });
            }}
            aria-label="P.O. number"
          />
        )}
        {dated && (
          <Select
            label="Terms"
            data={terms.filter((x) => x.is_active || x.id === draft.terms_id).map((x) => ({ value: x.id, label: x.name }))}
            value={draft.terms_id}
            onChange={(id) => update((d) => applySalesTerms(d, terms.find((x) => x.id === id) ?? null))}
            clearable
            searchable
            aria-label="Terms"
          />
        )}
        {dated && <DateField label="Due Date" value={draft.due_date} onChange={(due_date) => update({ due_date })} aria-label="Due date" />}
        {reps.length > 0 && (
          <Select
            label="Rep"
            data={reps.map((r) => ({ value: r.id, label: r.initials }))}
            value={draft.sales_rep_id}
            onChange={(sales_rep_id) => update({ sales_rep_id })}
            clearable
            aria-label="Sales rep"
          />
        )}
        {(invoice || t === 'SalesOrder') && <DateField label="Ship Date" value={draft.ship_date} onChange={(ship_date) => update({ ship_date })} aria-label="Ship date" />}
        {receipt && (
          <Select
            label="Payment Method"
            data={methods.filter((m) => m.is_active || m.id === draft.payment_method_id).map((m) => ({ value: m.id, label: m.name }))}
            value={draft.payment_method_id}
            onChange={(payment_method_id) => update({ payment_method_id })}
            clearable
            searchable
            aria-label="Payment method"
          />
        )}
        {receipt && (
          <AccountQuickFill
            label="Deposit To"
            types={['Bank', 'OtherCurrentAsset']}
            value={draft.account_id}
            onChange={(account_id) => update({ account_id })}
            placeholder="Undeposited Funds"
            allowAddNew={false}
            aria-label="Deposit to"
          />
        )}
      </div>
    </>
  );

  const taxLabel = taxItem ? `${taxItem.name} (${taxRate(taxItem, items) ?? '0'}%)` : 'Tax';
  const totalsBox = (
    <div className={mobile ? undefined : classes.headerGrid} style={mobile ? undefined : { gridTemplateColumns: '1fr 1fr 320px' }}>
      {charge ? (
        <span />
      ) : (
        <Textarea
          label="Customer Message"
          value={draft.message}
          onChange={(e) => {
            const message = e.currentTarget.value;
            update({ message });
          }}
          autosize
          minRows={2}
          aria-label="Customer message"
        />
      )}
      <div>
        <TextInput
          label="Memo"
          value={draft.memo}
          onChange={(e) => {
            const memo = e.currentTarget.value;
            update({ memo });
          }}
          aria-label="Memo"
        />
        {!charge && (
          <Group gap="md" mt="xs">
            <Checkbox size="xs" label="To be printed" checked={draft.to_be_printed} onChange={(e) => update({ to_be_printed: e.currentTarget.checked })} />
            <Checkbox size="xs" label="To be e-mailed" checked={draft.to_be_emailed} onChange={(e) => update({ to_be_emailed: e.currentTarget.checked })} />
          </Group>
        )}
      </div>
      <div>
        {salesTax && (
          <Group gap="xs" grow mb="xs">
            <Select
              label="Tax"
              size="xs"
              data={taxItems.filter((i) => i.is_active || i.id === draft.sales_tax_item_id).map((i) => ({ value: i.id, label: i.name }))}
              value={draft.sales_tax_item_id}
              onChange={(id) => {
                update({ sales_tax_item_id: id });
                if (id) localStorage.setItem(TAX_ITEM_KEY, id);
              }}
              clearable
              aria-label="Sales tax item"
            />
            <Select
              label="Customer Tax Code"
              size="xs"
              data={codeList.filter((c) => c.is_active || c.id === draft.customer_tax_code_id).map((c) => ({ value: c.id, label: c.name }))}
              value={draft.customer_tax_code_id}
              onChange={(customer_tax_code_id) => update({ customer_tax_code_id })}
              clearable
              aria-label="Customer tax code"
            />
          </Group>
        )}
        <div className={classes.totals} data-testid="sales-totals">
          <span className={classes.label}>Subtotal</span>
          <span className={classes.value}>{formatMoney(totals.subtotal)}</span>
          {salesTax && (
            <>
              <span className={classes.label}>{taxLabel}</span>
              <span className={classes.value}>{formatMoney(tax)}</span>
            </>
          )}
          <span className={`${classes.label} ${classes.grand}`}>Total</span>
          <span className={`${classes.value} ${classes.grand}`} aria-label="Total">
            {formatMoney(total)}
          </span>
        </div>
        {preview.data?.warnings.length ? (
          <Text size="xs" c="orange" mt={4}>
            {preview.data.warnings.map(warningText).join(' ')}
          </Text>
        ) : null}
        {!nonPosting && (
          <Checkbox size="xs" mt="xs" label="Show postings" checked={showPostings} onChange={(e) => setShowPostings(e.currentTarget.checked)} />
        )}
      </div>
    </div>
  );

  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid={`form-${SALES_FORM_SLUG[t]}`}>
      {form.banner}
      {header}
      {form.formErrors}
      <LineGrid
        label="Item lines"
        rows={draft.lines}
        onChange={setLines}
        columns={columns}
        rowKey={(r) => r.key}
        isBlank={isBlankSale}
        makeBlank={blankSaleRow}
        errors={gridErrors(errors, 'lines')}
        enterBehavior={enterBehavior}
        onEnterSave={onEnterSave}
        mobile={mobile}
        minVisibleRows={5}
      />
      {totalsBox}
      {saved && isOrderType(saved.txn_type) && <OrderProgressPanel order={saved} dirty={form.dirty} onDraw={(_target, progress) => setDrawing({ order: saved, progress })} />}
      <Collapse expanded={showPostings}>
        <PostingsPanel postings={preview.data?.postings ?? []} accounts={accounts} error={preview.error ? String((preview.error as Error).message) : null} />
      </Collapse>
      <TxnFormActions form={form} hideVoid={nonPosting} />
      {form.chrome}
      <CreateFromOrderModal order={drawing?.order ?? null} progress={drawing?.progress ?? null} target="Invoice" onClose={() => setDrawing(null)} onCreate={onDrawn} />
      <OrderPickerModal
        opened={pickOrder}
        type="SalesOrder"
        entityId={draft.entity_id}
        multiple={false}
        onClose={() => setPickOrder(false)}
        onPick={([picked]) => {
          setPickOrder(false);
          if (picked) setDrawing(picked);
        }}
      />
    </div>
  );
}

/** What saving will post, from the server's preview: the live Transaction Journal. */
export function PostingsPanel({ postings, accounts, error }: { postings: ReadonlyArray<{ account_id: string; amount: number; memo: string | null }>; accounts: Map<string, { full_name: string }>; error: string | null }) {
  if (error) return <Text size="sm" c="red">{error}</Text>;
  if (!postings.length) return <Text size="sm" c="dimmed">Nothing to post yet.</Text>;
  return (
    <Table fz="sm" withTableBorder>
      <Table.Thead>
        <Table.Tr>
          <Table.Th>Account</Table.Th>
          <Table.Th>Memo</Table.Th>
          <Table.Th ta="right">Debit</Table.Th>
          <Table.Th ta="right">Credit</Table.Th>
        </Table.Tr>
      </Table.Thead>
      <Table.Tbody>
        {postings.map((p, i) => (
          <Table.Tr key={i}>
            <Table.Td>{accounts.get(p.account_id)?.full_name ?? p.account_id}</Table.Td>
            <Table.Td>{p.memo}</Table.Td>
            <Table.Td ta="right">{p.amount > 0 ? formatMoney(p.amount) : ''}</Table.Td>
            <Table.Td ta="right">{p.amount < 0 ? formatMoney(-p.amount) : ''}</Table.Td>
          </Table.Tr>
        ))}
      </Table.Tbody>
    </Table>
  );
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
