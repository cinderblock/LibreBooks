// New Item / Edit Item (Item List: Ctrl+N, Ctrl+E). QBD's dialog changes face
// with the type: a service or part has a sale side (description, price, income
// account, tax code) and, when it's two-sided, a purchase side (description,
// cost, expense or COGS account, preferred vendor); an inventory part adds the
// asset account and quantities; a discount or other charge can be a percent; a
// group or sales tax group lists its members; a payment item says where the
// money lands; a sales tax item names its rate and agency.

import { Checkbox, Group, SegmentedControl, Select, SimpleGrid, Stack, Text, Textarea, TextInput, Title } from '@mantine/core';
import { useMemo, useRef } from 'react';
import { blankItem } from '~/api/blank';
import { useCompanyInfo, useItemIndex, useItems, useSalesTaxCodes } from '~/api/hooks';
import type { Item, ItemComponent, ItemType } from '~/api/types';
import { LineGrid, withTrailingBlank, type GridColumn } from '~/inputs/LineGrid';
import { AccountQuickFill, itemOptions, NameQuickFill } from '~/inputs/pickers';
import { QuickFill } from '~/inputs/QuickFill';
import { ACCOUNT_TYPES } from '~/lib/accountTypes';
import { ITEM_TYPE_LABEL } from '~/lib/txnTypes';
import { paths } from '~/windows/paths';
import { gridErrors, noErrors, putGridError, type FormErrors } from '../model';
import { RecordFormActions, useRecordForm, type RecordFormSpec } from '../useRecordForm';
import classes from '../forms.module.css';
import { decimalError, decimalOrNull, fullNameOf, possibleParents, recordServerErrors, requireName, textOrNull } from './model';

export interface ComponentRow {
  key: string;
  item_id: string | null;
  quantity: string;
}

export interface ItemDraft {
  item: Item;
  /** Group and sales tax group members, with the grid's trailing blank line. */
  rows: ComponentRow[];
  /** Payment items: deposit to an account (true) or group with other undeposited funds (false). */
  depositTo: boolean;
}

/** The types the editor offers for a new item (assemblies and fixed assets wait for their features). */
export const ITEM_TYPES: readonly ItemType[] = ['Service', 'Inventory', 'NonInventory', 'OtherCharge', 'Subtotal', 'Group', 'Discount', 'Payment', 'SalesTax', 'SalesTaxGroup'];
const isItemType = (t: string | undefined): t is ItemType => !!t && t in ITEM_TYPE_LABEL;

const POSTING_TYPES = ACCOUNT_TYPES.filter((t) => t !== 'NonPosting');
const COGS_TYPES = ACCOUNT_TYPES.filter((t) => t === 'CostOfGoodsSold' || t === 'Expense' || t === 'OtherExpense');
const ASSET_TYPES = ACCOUNT_TYPES.filter((t) => t === 'OtherCurrentAsset' || t === 'OtherAsset' || t === 'FixedAsset');
const DEPOSIT_TYPES = ACCOUNT_TYPES.filter((t) => t === 'Bank' || t === 'OtherCurrentAsset');

export const blankComponentRow = (): ComponentRow => ({ key: crypto.randomUUID(), item_id: null, quantity: '1' });
export const isBlankComponent = (r: ComponentRow) => !r.item_id;

/** Which sides of the item its type has. */
export function itemSides(t: ItemType): { sale: boolean; purchase: 'always' | 'optional' | 'never'; components: boolean } {
  switch (t) {
    case 'Service':
    case 'NonInventory':
    case 'OtherCharge':
      return { sale: true, purchase: 'optional', components: false };
    case 'Inventory':
    case 'InventoryAssembly':
      return { sale: true, purchase: 'always', components: t === 'InventoryAssembly' };
    case 'Group':
    case 'SalesTaxGroup':
      return { sale: false, purchase: 'never', components: true };
    case 'FixedAsset':
      return { sale: false, purchase: 'always', components: false };
    default:
      return { sale: false, purchase: 'never', components: false };
  }
}

export function validateItem(d: ItemDraft): FormErrors {
  const e = noErrors();
  const it = d.item;
  const t = it.item_type;
  const sides = itemSides(t);
  requireName(e, it.name, 'an item name');
  const dec = (key: 'sales_price' | 'purchase_cost' | 'reorder_point' | 'quantity_on_hand' | 'tax_rate') => {
    const err = decimalError(it[key]);
    if (err) e.header[key] = err;
  };
  dec('sales_price');
  dec('purchase_cost');
  dec('reorder_point');
  dec('quantity_on_hand');
  dec('tax_rate');
  const twoSided = sides.purchase === 'always' || (sides.purchase === 'optional' && it.has_purchase_info);
  if (sides.sale && !it.income_account_id) e.header.income_account_id = twoSided ? 'Choose the income account.' : 'Choose an account.';
  if (twoSided && !it.expense_account_id) e.header.expense_account_id = t === 'Inventory' ? 'Choose the COGS account.' : 'Choose the expense account.';
  if (t === 'Inventory' && !it.asset_account_id) e.header.asset_account_id = 'Choose the inventory asset account.';
  if (t === 'Discount' && !it.income_account_id) e.header.income_account_id = 'Choose the account the discount posts to.';
  if (t === 'Payment' && d.depositTo && !it.deposit_account_id) e.header.deposit_account_id = 'Choose the account to deposit to.';
  if (t === 'SalesTax') {
    if (!it.tax_rate?.trim()) e.header.tax_rate = 'Enter the tax rate.';
    if (!it.tax_agency_id) e.header.tax_agency_id = 'Choose the tax agency (a vendor).';
  }
  if (sides.components) {
    d.rows.forEach((r, i) => {
      if (isBlankComponent(r)) return;
      if (t === 'Group' && decimalError(r.quantity)) putGridError(e, 'components', i, 'quantity', 'Not a number');
      if (r.item_id === it.id) putGridError(e, 'components', i, 'item', "A group can't contain itself.");
    });
  }
  return e;
}

export function itemToRecord(d: ItemDraft): Item {
  const it = d.item;
  const sides = itemSides(it.item_type);
  const twoSided = sides.purchase === 'always' || (sides.purchase === 'optional' && it.has_purchase_info);
  const components: ItemComponent[] = sides.components ? d.rows.filter((r) => !isBlankComponent(r)).map((r) => ({ item_id: r.item_id!, quantity: decimalOrNull(r.quantity) ?? '1' })) : [];
  return {
    ...it,
    name: it.name.trim(),
    sku: textOrNull(it.sku),
    unit_of_measure: textOrNull(it.unit_of_measure),
    sales_description: textOrNull(it.sales_description),
    sales_price: decimalOrNull(it.sales_price),
    purchase_description: twoSided ? textOrNull(it.purchase_description) : null,
    purchase_cost: twoSided ? decimalOrNull(it.purchase_cost) : null,
    expense_account_id: twoSided ? it.expense_account_id : null,
    preferred_vendor_id: twoSided ? it.preferred_vendor_id : null,
    has_purchase_info: twoSided,
    reorder_point: decimalOrNull(it.reorder_point),
    quantity_on_hand: decimalOrNull(it.quantity_on_hand) ?? '0',
    tax_rate: it.item_type === 'SalesTax' ? decimalOrNull(it.tax_rate) : null,
    deposit_account_id: it.item_type === 'Payment' && d.depositTo ? it.deposit_account_id : null,
    components,
  };
}

function draftFromItem(item: Item): ItemDraft {
  const rows = item.components.map<ComponentRow>((c) => ({ key: crypto.randomUUID(), item_id: c.item_id, quantity: c.quantity }));
  return { item, rows: withTrailingBlank(rows, isBlankComponent, blankComponentRow), depositTo: item.deposit_account_id !== null };
}

const spec: RecordFormSpec<'items', ItemDraft> = {
  list: 'items',
  noun: 'item',
  newDraft: (params) => draftFromItem(blankItem('', isItemType(params.type) ? params.type : 'Service')),
  fromRecord: draftFromItem,
  toRecord: itemToRecord,
  validate: validateItem,
  isEmpty: (d) => !d.item.name.trim() && !d.item.sales_description && !d.item.purchase_description && !d.item.income_account_id && !d.item.expense_account_id && d.rows.every(isBlankComponent),
  fingerprint: (d) => JSON.stringify(itemToRecord(d)),
  serverErrors: (_d, err) => recordServerErrors(err, { parent_id: 'parent', item_type: 'type' }),
  // Save & New keeps the type (QBD's dialog does).
  nextDraft: (d) => draftFromItem(blankItem('', d.item.item_type)),
  newPath: (d) => paths.newRecord('items', { type: d.item.item_type }),
  title: (_d, saved) => (saved ? `Edit Item: ${saved.full_name}` : 'New Item'),
  savedMessage: (r) => `Item "${r.full_name}" saved.`,
  attachmentKind: 'item',
};

export function ItemEditor({ saved, params }: { saved: Item | null; params: Record<string, string> }) {
  const { data: items = [] } = useItems();
  const itemIndex = useItemIndex();
  const { data: taxCodes = [] } = useSalesTaxCodes();
  const { data: info } = useCompanyInfo();
  const firstField = useRef<HTMLInputElement>(null);
  const form = useRecordForm({ spec, saved, params, onSavedNew: () => firstField.current?.focus() });
  const { draft, update, errors, mobile } = form;
  const it = draft.item;
  const t = it.item_type;
  const sides = itemSides(t);
  const setItem = (patch: Partial<Item>) => update((d) => ({ ...d, item: { ...d.item, ...patch } }));
  const salesTax = info?.preferences.sales_tax_enabled ?? true;
  const twoSided = sides.purchase === 'always' || (sides.purchase === 'optional' && it.has_purchase_info);

  // A subitem is of its parent's type, and never inside itself.
  const parentOptions = useMemo(() => itemOptions(possibleParents(items.filter((i) => i.item_type === t), saved), 'sell'), [items, t, saved]);
  const parent = it.parent_id ? itemIndex.get(it.parent_id) : undefined;
  // Members of a group: anything but groups; of a sales tax group: sales tax items.
  const memberOptions = useMemo(() => itemOptions(items.filter((i) => i.id !== it.id && (t === 'SalesTaxGroup' ? i.item_type === 'SalesTax' : i.item_type !== 'Group' && i.item_type !== 'SalesTaxGroup')), 'sell'), [items, t, it.id]);
  const taxCodeOptions = taxCodes.filter((c) => c.is_active || c.id === it.sales_tax_code_id).map((c) => ({ value: c.id, label: `${c.name}${c.description ? ` · ${c.description}` : ''}` }));

  const componentColumns = useMemo<GridColumn<ComponentRow>[]>(
    () => [
      {
        key: 'item',
        header: t === 'SalesTaxGroup' ? 'Tax item' : 'Item',
        width: '60%',
        render: (c) => (
          <QuickFill
            variant={c.variant === 'cell' ? 'cell' : 'default'}
            label={c.variant === 'default' ? c.label : undefined}
            size="xs"
            options={memberOptions}
            columnHeaders={['Type', 'Price']}
            value={c.row.item_id}
            onChange={(id) => c.update((r) => ({ ...r, item_id: id }))}
            onCommit={c.onCommit}
            error={c.error}
            inputProps={c.cellProps}
          />
        ),
      },
      {
        key: 'description',
        header: 'Description',
        render: (c) => (
          <Text size="xs" c="dimmed" px={4} truncate>
            {c.row.item_id ? (itemIndex.get(c.row.item_id)?.sales_description ?? '') : ''}
          </Text>
        ),
        mobile: 'hidden',
      },
      ...(t === 'Group'
        ? [
            {
              key: 'quantity',
              header: 'Qty',
              width: 90,
              align: 'right' as const,
              render: (c) => <TextInput variant={c.variant === 'cell' ? 'unstyled' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" value={c.row.quantity} onChange={(e) => c.update((r) => ({ ...r, quantity: e.currentTarget.value }))} error={c.error} inputMode="decimal" styles={{ input: { textAlign: 'right' } }} {...c.cellProps} />,
              mobile: 'half' as const,
            } satisfies GridColumn<ComponentRow>,
          ]
        : []),
    ],
    [t, memberOptions, itemIndex],
  );

  const field = (key: 'sales_description' | 'purchase_description', label: string) => <Textarea label={label} value={it[key] ?? ''} onChange={(e) => setItem({ [key]: e.currentTarget.value })} autosize minRows={2} aria-label={label} />;
  const rate = (key: 'sales_price' | 'purchase_cost' | 'reorder_point' | 'quantity_on_hand' | 'tax_rate', label: string, disabled = false) => (
    <TextInput label={label} value={it[key] ?? ''} onChange={(e) => setItem({ [key]: e.currentTarget.value })} error={errors.header[key]} inputMode="decimal" disabled={disabled} styles={{ input: { textAlign: 'right' } }} aria-label={label} />
  );
  const taxCode = salesTax && <Select label="Tax code" data={taxCodeOptions} value={it.sales_tax_code_id} onChange={(sales_tax_code_id) => setItem({ sales_tax_code_id })} clearable aria-label="Tax code" />;
  const percentToggle = (label: string) => (
    <div>
      <Text size="sm" fw={500} mb={4}>
        {label}
      </Text>
      <SegmentedControl size="xs" data={[{ value: 'amount', label: 'Amount' }, { value: 'percent', label: 'Percent' }]} value={it.is_percent ? 'percent' : 'amount'} onChange={(v) => setItem({ is_percent: v === 'percent' })} />
    </div>
  );

  let body: React.ReactNode;
  switch (t) {
    case 'Service':
    case 'NonInventory':
    case 'OtherCharge':
    case 'Inventory':
      body = (
        <Stack gap="sm">
          {sides.purchase === 'optional' && (
            <Checkbox
              label={t === 'Service' ? 'This service is used in assemblies or is performed by a subcontractor or partner' : 'This item is used in assemblies or is purchased for a specific customer:job'}
              description="Two-sided: a purchase side with its cost and expense account, and a sale side with its price and income account."
              checked={it.has_purchase_info}
              onChange={(e) => setItem({ has_purchase_info: e.currentTarget.checked })}
            />
          )}
          <SimpleGrid cols={{ base: 1, md: twoSided ? 2 : 1 }} spacing="md">
            {twoSided && (
              <Stack gap="sm" className={mobile ? undefined : classes.headerGrid} style={{ display: 'flex' }}>
                <Text fw={600} size="sm">
                  Purchase information
                </Text>
                {field('purchase_description', 'Description on purchase transactions')}
                <SimpleGrid cols={2}>
                  {rate('purchase_cost', 'Cost')}
                  <AccountQuickFill label={t === 'Inventory' ? 'COGS account' : 'Expense account'} types={t === 'Inventory' ? COGS_TYPES : POSTING_TYPES} value={it.expense_account_id} onChange={(expense_account_id) => setItem({ expense_account_id })} error={errors.header.expense_account_id} aria-label={t === 'Inventory' ? 'COGS account' : 'Expense account'} />
                </SimpleGrid>
                <NameQuickFill label="Preferred vendor" kinds={['Vendor']} value={it.preferred_vendor_id} onChange={(preferred_vendor_id) => setItem({ preferred_vendor_id })} aria-label="Preferred vendor" />
              </Stack>
            )}
            <Stack gap="sm" className={mobile ? undefined : classes.headerGrid} style={{ display: 'flex' }}>
              <Text fw={600} size="sm">
                {twoSided ? 'Sales information' : 'Sale'}
              </Text>
              {field('sales_description', twoSided ? 'Description on sales transactions' : 'Description')}
              <SimpleGrid cols={2}>
                {t === 'OtherCharge' && percentToggle('Charge as')}
                {rate('sales_price', t === 'OtherCharge' ? (it.is_percent ? 'Percent (%)' : 'Amount') : t === 'Service' ? 'Rate' : 'Sales price')}
                {taxCode}
              </SimpleGrid>
              <AccountQuickFill label={twoSided ? 'Income account' : 'Account'} types={POSTING_TYPES} value={it.income_account_id} onChange={(income_account_id) => setItem({ income_account_id })} error={errors.header.income_account_id} aria-label={twoSided ? 'Income account' : 'Account'} />
            </Stack>
          </SimpleGrid>
          {t === 'Inventory' && (
            <Stack gap="sm" className={mobile ? undefined : classes.headerGrid} style={{ display: 'flex' }}>
              <Text fw={600} size="sm">
                Inventory information
              </Text>
              <SimpleGrid cols={{ base: 1, sm: 4 }}>
                <AccountQuickFill label="Asset account" types={ASSET_TYPES} value={it.asset_account_id} onChange={(asset_account_id) => setItem({ asset_account_id })} error={errors.header.asset_account_id} aria-label="Asset account" />
                {rate('reorder_point', 'Reorder point')}
                {rate('quantity_on_hand', saved ? 'On hand (from transactions)' : 'On hand', !!saved)}
                <TextInput label="Average cost" value={it.average_cost} disabled aria-label="Average cost" />
              </SimpleGrid>
            </Stack>
          )}
        </Stack>
      );
      break;
    case 'Subtotal':
      body = (
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            A subtotal adds up the lines above it on a sales form, so a discount or charge below can apply to them.
          </Text>
          {field('sales_description', 'Description')}
        </Stack>
      );
      break;
    case 'Discount':
      body = (
        <Stack gap="sm">
          {field('sales_description', 'Description')}
          <SimpleGrid cols={{ base: 1, sm: 3 }}>
            {percentToggle('Discount as')}
            {rate('sales_price', it.is_percent ? 'Percent (%)' : 'Amount')}
            {taxCode}
          </SimpleGrid>
          <AccountQuickFill label="Account" types={POSTING_TYPES} value={it.income_account_id} onChange={(income_account_id) => setItem({ income_account_id })} error={errors.header.income_account_id} aria-label="Account" />
          <Text size="xs" c="dimmed">
            A discount applies to the line above it (or a subtotal). Applies before sales tax when the tax code is taxable.
          </Text>
        </Stack>
      );
      break;
    case 'Payment':
      body = (
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            A payment item on an invoice records money received at the time of sale and reduces the amount due.
          </Text>
          {field('sales_description', 'Description')}
          <SegmentedControl
            data={[
              { value: 'group', label: 'Group with other undeposited funds' },
              { value: 'deposit', label: 'Deposit to an account' },
            ]}
            value={draft.depositTo ? 'deposit' : 'group'}
            onChange={(v) => update({ depositTo: v === 'deposit' })}
          />
          {draft.depositTo && <AccountQuickFill label="Deposit to" types={DEPOSIT_TYPES} value={it.deposit_account_id} onChange={(deposit_account_id) => setItem({ deposit_account_id })} error={errors.header.deposit_account_id} aria-label="Deposit to" />}
        </Stack>
      );
      break;
    case 'SalesTax':
      body = (
        <Stack gap="sm">
          {field('sales_description', 'Description (prints on forms)')}
          <SimpleGrid cols={{ base: 1, sm: 2 }}>
            {rate('tax_rate', 'Tax rate (%)')}
            <NameQuickFill label="Tax agency (the vendor you pay)" kinds={['Vendor']} value={it.tax_agency_id} onChange={(tax_agency_id) => setItem({ tax_agency_id })} error={errors.header.tax_agency_id} aria-label="Tax agency" />
          </SimpleGrid>
        </Stack>
      );
      break;
    case 'Group':
    case 'SalesTaxGroup':
      body = (
        <Stack gap="sm">
          {field('sales_description', 'Description')}
          {t === 'Group' && <Checkbox label="Print items in group on forms" checked={it.print_components} onChange={(e) => setItem({ print_components: e.currentTarget.checked })} />}
          <LineGrid label={t === 'Group' ? 'Group items' : 'Sales tax items in this group'} rows={draft.rows} onChange={(rows) => update({ rows })} columns={componentColumns} rowKey={(r) => r.key} isBlank={isBlankComponent} makeBlank={blankComponentRow} errors={gridErrors(errors, 'components')} mobile={mobile} minVisibleRows={4} />
        </Stack>
      );
      break;
    default:
      body = (
        <Text size="sm" c="dimmed">
          {ITEM_TYPE_LABEL[t]} items can be renamed and made inactive here; their other fields wait for their features.
        </Text>
      );
  }

  return (
    <div className={mobile ? classes.mobileForm : classes.form} style={mobile ? undefined : { maxWidth: 900 }} data-testid="record-item" {...form.enterProps}>
      {form.banner}
      <Group justify="space-between" align="flex-start" wrap="wrap">
        <Title order={4}>{saved ? 'Edit Item' : 'New Item'}</Title>
        <Checkbox label="Item is inactive" checked={!it.is_active} onChange={(e) => setItem({ is_active: !e.currentTarget.checked })} />
      </Group>
      <SimpleGrid cols={{ base: 1, sm: 3 }}>
        <Select
          label="Type"
          data={(saved && !ITEM_TYPES.includes(t) ? [t, ...ITEM_TYPES] : ITEM_TYPES).map((x) => ({ value: x, label: ITEM_TYPE_LABEL[x] }))}
          value={t}
          onChange={(v) => {
            if (!isItemType(v ?? undefined)) return;
            // A fresh item of the new type keeps what was typed so far.
            update((d) => ({ ...d, item: { ...blankItem(d.item.name, v as ItemType), id: d.item.id, sku: d.item.sku, sales_description: d.item.sales_description, is_active: d.item.is_active }, rows: withTrailingBlank([], isBlankComponent, blankComponentRow) }));
          }}
          allowDeselect={false}
          disabled={!!saved}
          description={saved ? "An item's type is fixed once it's used; make a new item instead." : undefined}
          error={errors.header.type}
          aria-label="Item type"
        />
        <TextInput
          label="Item name / number"
          value={it.name}
          onChange={(e) => {
            const name = e.currentTarget.value;
            update((d) => ({ ...d, item: { ...d.item, name, full_name: fullNameOf(name, parent) } }));
          }}
          required
          error={errors.header.name}
          ref={firstField}
          autoFocus={!mobile}
          aria-label="Item name"
        />
        <QuickFill
          label="Subitem of"
          placeholder="(top level)"
          options={parentOptions}
          columnHeaders={['Type', 'Price']}
          value={it.parent_id}
          onChange={(id) => {
            const p = id ? itemIndex.get(id) : undefined;
            setItem({ parent_id: p?.id ?? null, sublevel: p ? p.sublevel + 1 : 0, full_name: fullNameOf(it.name, p) });
          }}
          error={errors.header.parent}
          aria-label="Subitem of"
        />
      </SimpleGrid>
      {(sides.sale || sides.purchase !== 'never') && (
        <SimpleGrid cols={{ base: 1, sm: 3 }}>
          <TextInput label="SKU / part number" value={it.sku ?? ''} onChange={(e) => setItem({ sku: e.currentTarget.value })} aria-label="SKU" />
          <TextInput label="Unit of measure" placeholder="each, hour, board foot…" value={it.unit_of_measure ?? ''} onChange={(e) => setItem({ unit_of_measure: e.currentTarget.value })} aria-label="Unit of measure" />
        </SimpleGrid>
      )}
      {body}
      {form.formErrors}
      <RecordFormActions form={form} />
      {form.chrome}
    </div>
  );
}
