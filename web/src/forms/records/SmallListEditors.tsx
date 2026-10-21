// The small lists' editors (Lists menu: Ctrl+N, Ctrl+E): classes (a name and
// a parent), terms (QBD's two kinds: standard "net N days, M% if paid in D",
// and date-driven "due on the Nth of the month"), payment methods (a name and
// its kind) and sales tax codes (a short code, taxable or not).

import { Checkbox, Group, NumberInput, Select, SegmentedControl, SimpleGrid, Stack, Text, TextInput, Title } from '@mantine/core';
import { useMemo, useRef } from 'react';
import { blankClass, blankPaymentMethod, blankSalesTaxCode, blankTerms } from '~/api/blank';
import { useClasses } from '~/api/hooks';
import type { Class, PaymentMethod, PaymentMethodKind, SalesTaxCode, Terms, TermsKind } from '~/api/types';
import { QuickFill } from '~/inputs/QuickFill';
import { paths } from '~/windows/paths';
import { noErrors, type FormErrors } from '../model';
import { RecordFormActions, useRecordForm, type RecordForm, type RecordFormSpec } from '../useRecordForm';
import classes from '../forms.module.css';
import { decimalError, decimalOrNull, fullNameOf, possibleParents, recordServerErrors, requireName, textOrNull, wholeNumber } from './model';

const PAYMENT_KIND_LABEL: Record<PaymentMethodKind, string> = {
  Cash: 'Cash',
  Check: 'Check',
  AmericanExpress: 'American Express',
  Discover: 'Discover',
  MasterCard: 'MasterCard',
  Visa: 'Visa',
  DebitCard: 'Debit card',
  GiftCard: 'Gift card',
  ECheck: 'E-check / ACH',
  OtherCreditCard: 'Other credit card',
  Other: 'Other',
};

function Shell<L extends 'classes' | 'terms' | 'payment-methods' | 'sales-tax-codes', D>({ form, title, testId, inactiveLabel, inactive, onInactive, children, maxWidth = 560 }: { form: RecordForm<L, D>; title: string; testId: string; inactiveLabel: string; inactive: boolean; onInactive(v: boolean): void; children: React.ReactNode; maxWidth?: number }) {
  const { mobile } = form;
  return (
    <div className={mobile ? classes.mobileForm : classes.form} style={mobile ? undefined : { maxWidth }} data-testid={testId} {...form.enterProps}>
      {form.banner}
      <Group justify="space-between" align="flex-start" wrap="wrap">
        <Title order={4}>{title}</Title>
        <Checkbox label={inactiveLabel} checked={inactive} onChange={(e) => onInactive(e.currentTarget.checked)} />
      </Group>
      {children}
      {form.formErrors}
      <RecordFormActions form={form} />
      {form.chrome}
    </div>
  );
}

// ---- classes -----------------------------------------------------------------

const classSpec: RecordFormSpec<'classes', Class> = {
  list: 'classes',
  noun: 'class',
  newDraft: () => blankClass(''),
  fromRecord: (r) => r,
  toRecord: (d) => ({ ...d, name: d.name.trim() }),
  validate: (d) => {
    const e = noErrors();
    requireName(e, d.name, 'a class name');
    return e;
  },
  isEmpty: (d) => !d.name.trim() && !d.parent_id,
  fingerprint: (d) => JSON.stringify(classSpec.toRecord(d)),
  serverErrors: (_d, err) => recordServerErrors(err, { parent_id: 'parent' }),
  nextDraft: () => blankClass(''),
  newPath: () => paths.newRecord('classes'),
  title: (_d, saved) => (saved ? `Edit Class: ${saved.full_name}` : 'New Class'),
  savedMessage: (r) => `Class "${r.full_name}" saved.`,
};

export function ClassEditor({ saved, params }: { saved: Class | null; params: Record<string, string> }) {
  const { data: all = [] } = useClasses();
  const firstField = useRef<HTMLInputElement>(null);
  const form = useRecordForm({ spec: classSpec, saved, params, onSavedNew: () => firstField.current?.focus() });
  const { draft: d, update, errors, mobile } = form;
  const parents = useMemo(() => possibleParents(all, saved).map((c) => ({ id: c.id, fullName: c.full_name, name: c.name, depth: c.sublevel, inactive: !c.is_active })), [all, saved]);
  const parent = d.parent_id ? all.find((c) => c.id === d.parent_id) : undefined;
  return (
    <Shell form={form} title={saved ? 'Edit Class' : 'New Class'} testId="record-class" inactiveLabel="Class is inactive" inactive={!d.is_active} onInactive={(v) => update({ is_active: !v })}>
      <TextInput label="Class name" value={d.name} onChange={(e) => update((x) => ({ ...x, name: e.currentTarget.value, full_name: fullNameOf(e.currentTarget.value, parent) }))} required error={errors.header.name} ref={firstField} autoFocus={!mobile} aria-label="Class name" />
      <QuickFill
        label="Subclass of"
        placeholder="(top level)"
        options={parents}
        value={d.parent_id}
        onChange={(id) => {
          const p = id ? all.find((c) => c.id === id) : undefined;
          update({ parent_id: p?.id ?? null, sublevel: p ? p.sublevel + 1 : 0, full_name: fullNameOf(d.name, p) });
        }}
        error={errors.header.parent}
        aria-label="Subclass of"
      />
    </Shell>
  );
}

// ---- terms -------------------------------------------------------------------

export function validateTerms(d: Terms): FormErrors {
  const e = noErrors();
  requireName(e, d.name, 'a name for the terms');
  const k = d.kind;
  const pct = decimalError(k.discount_percent);
  if (pct) e.header.discount_percent = pct;
  if (k.type === 'Standard') {
    if (k.discount_days !== null && k.discount_days > k.due_days) e.header.discount_days = 'The discount period ends after the due date.';
    if (k.discount_percent && k.discount_days === null) e.header.discount_days = 'Say within how many days the discount applies.';
  } else {
    if (k.day_of_month < 1 || k.day_of_month > 31) e.header.day_of_month = 'A day of the month, 1–31.';
    if (k.discount_day_of_month !== null && (k.discount_day_of_month < 1 || k.discount_day_of_month > 31)) e.header.discount_day_of_month = 'A day of the month, 1–31.';
    if (k.discount_percent && k.discount_day_of_month === null) e.header.discount_day_of_month = 'Say by which day the discount applies.';
  }
  return e;
}

export function termsToRecord(d: Terms): Terms {
  const k = d.kind;
  const discount_percent = decimalOrNull(k.discount_percent);
  const kind: TermsKind = k.type === 'Standard' ? { type: 'Standard', due_days: k.due_days, discount_days: discount_percent ? k.discount_days : null, discount_percent } : { type: 'DateDriven', day_of_month: k.day_of_month, cutoff_days: k.cutoff_days, discount_day_of_month: discount_percent ? k.discount_day_of_month : null, discount_percent };
  return { ...d, name: d.name.trim(), kind };
}

const termsSpec: RecordFormSpec<'terms', Terms> = {
  list: 'terms',
  noun: 'terms',
  newDraft: () => blankTerms(''),
  fromRecord: (r) => r,
  toRecord: termsToRecord,
  validate: validateTerms,
  isEmpty: (d) => !d.name.trim() && JSON.stringify(d.kind) === JSON.stringify(blankTerms('').kind),
  fingerprint: (d) => JSON.stringify(termsToRecord(d)),
  serverErrors: (_d, err) => recordServerErrors(err),
  nextDraft: () => blankTerms(''),
  newPath: () => paths.newRecord('terms'),
  title: (_d, saved) => (saved ? `Edit Terms: ${saved.name}` : 'New Terms'),
  savedMessage: (r) => `Terms "${r.name}" saved.`,
};

export function TermsEditor({ saved, params }: { saved: Terms | null; params: Record<string, string> }) {
  const firstField = useRef<HTMLInputElement>(null);
  const form = useRecordForm({ spec: termsSpec, saved, params, onSavedNew: () => firstField.current?.focus() });
  const { draft: d, update, errors, mobile } = form;
  const k = d.kind;
  const setKind = (patch: Partial<TermsKind>) => update({ kind: { ...k, ...patch } as TermsKind });
  const switchKind = (type: 'Standard' | 'DateDriven') => {
    if (type === k.type) return;
    update({ kind: type === 'Standard' ? { type, due_days: 30, discount_days: null, discount_percent: k.discount_percent } : { type, day_of_month: 1, cutoff_days: 5, discount_day_of_month: null, discount_percent: k.discount_percent } });
  };
  const num = (label: string, value: number | null, onChange: (v: number | null) => void, opts: { min?: number; max?: number; error?: string; allowBlank?: boolean } = {}) => (
    <NumberInput label={label} value={value ?? ''} onChange={(v) => onChange(opts.allowBlank && v === '' ? null : (wholeNumber(v, opts.min ?? 0) ?? (opts.allowBlank ? null : 0)))} min={opts.min ?? 0} max={opts.max} allowDecimal={false} error={opts.error} aria-label={label} />
  );
  return (
    <Shell form={form} title={saved ? 'Edit Terms' : 'New Terms'} testId="record-terms" inactiveLabel="Terms are inactive" inactive={!d.is_active} onInactive={(v) => update({ is_active: !v })}>
      <TextInput label="Terms" placeholder="Net 30, 2% 10 Net 30, Due on receipt…" value={d.name} onChange={(e) => update({ name: e.currentTarget.value })} required error={errors.header.name} ref={firstField} autoFocus={!mobile} aria-label="Terms name" />
      <SegmentedControl fullWidth data={[{ value: 'Standard', label: 'Standard' }, { value: 'DateDriven', label: 'Date driven' }]} value={k.type} onChange={(v) => switchKind(v as 'Standard' | 'DateDriven')} />
      {k.type === 'Standard' ? (
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            Counted from the transaction date: due in N days, with a discount if paid within the first few.
          </Text>
          <SimpleGrid cols={{ base: 1, sm: 3 }}>
            {num('Net due in (days)', k.due_days, (v) => setKind({ due_days: v ?? 0 }), { error: errors.header.due_days })}
            <TextInput label="Discount percentage" value={k.discount_percent ?? ''} onChange={(e) => setKind({ discount_percent: e.currentTarget.value || null })} inputMode="decimal" error={errors.header.discount_percent} aria-label="Discount percentage" />
            {num('Discount if paid within (days)', k.discount_days, (v) => setKind({ discount_days: v }), { allowBlank: true, error: errors.header.discount_days })}
          </SimpleGrid>
        </Stack>
      ) : (
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            Due on a day of the month; a transaction dated within the cutoff days before it rolls to the next month.
          </Text>
          <SimpleGrid cols={{ base: 1, sm: 2 }}>
            {num('Net due before the (day of month)', k.day_of_month, (v) => setKind({ day_of_month: v ?? 1 }), { min: 1, max: 31, error: errors.header.day_of_month })}
            {num('Due the next month if issued within (days of due date)', k.cutoff_days, (v) => setKind({ cutoff_days: v ?? 0 }), { error: errors.header.cutoff_days })}
            <TextInput label="Discount percentage" value={k.discount_percent ?? ''} onChange={(e) => setKind({ discount_percent: e.currentTarget.value || null })} inputMode="decimal" error={errors.header.discount_percent} aria-label="Discount percentage" />
            {num('Discount if paid before the (day of month)', k.discount_day_of_month, (v) => setKind({ discount_day_of_month: v }), { min: 1, max: 31, allowBlank: true, error: errors.header.discount_day_of_month })}
          </SimpleGrid>
        </Stack>
      )}
    </Shell>
  );
}

// ---- payment methods ------------------------------------------------------------

const paymentMethodSpec: RecordFormSpec<'payment-methods', PaymentMethod> = {
  list: 'payment-methods',
  noun: 'payment method',
  newDraft: () => blankPaymentMethod(''),
  fromRecord: (r) => r,
  toRecord: (d) => ({ ...d, name: d.name.trim() }),
  validate: (d) => {
    const e = noErrors();
    requireName(e, d.name, 'a name for the payment method');
    return e;
  },
  isEmpty: (d) => !d.name.trim() && d.kind === 'Other',
  fingerprint: (d) => JSON.stringify(paymentMethodSpec.toRecord(d)),
  serverErrors: (_d, err) => recordServerErrors(err),
  nextDraft: () => blankPaymentMethod(''),
  newPath: () => paths.newRecord('payment-methods'),
  title: (_d, saved) => (saved ? `Edit Payment Method: ${saved.name}` : 'New Payment Method'),
  savedMessage: (r) => `Payment method "${r.name}" saved.`,
};

export function PaymentMethodEditor({ saved, params }: { saved: PaymentMethod | null; params: Record<string, string> }) {
  const firstField = useRef<HTMLInputElement>(null);
  const form = useRecordForm({ spec: paymentMethodSpec, saved, params, onSavedNew: () => firstField.current?.focus() });
  const { draft: d, update, errors, mobile } = form;
  return (
    <Shell form={form} title={saved ? 'Edit Payment Method' : 'New Payment Method'} testId="record-payment-method" inactiveLabel="Payment method is inactive" inactive={!d.is_active} onInactive={(v) => update({ is_active: !v })}>
      <TextInput label="Payment method" value={d.name} onChange={(e) => update({ name: e.currentTarget.value })} required error={errors.header.name} ref={firstField} autoFocus={!mobile} aria-label="Payment method name" />
      <Select label="Payment type" data={(Object.keys(PAYMENT_KIND_LABEL) as PaymentMethodKind[]).map((k) => ({ value: k, label: PAYMENT_KIND_LABEL[k] }))} value={d.kind} onChange={(v) => v && update({ kind: v as PaymentMethodKind })} allowDeselect={false} error={errors.header.kind} aria-label="Payment type" />
    </Shell>
  );
}

// ---- sales tax codes -------------------------------------------------------------

const salesTaxCodeSpec: RecordFormSpec<'sales-tax-codes', SalesTaxCode> = {
  list: 'sales-tax-codes',
  noun: 'sales tax code',
  newDraft: () => blankSalesTaxCode(''),
  fromRecord: (r) => r,
  toRecord: (d) => ({ ...d, name: d.name.trim(), description: textOrNull(d.description) }),
  validate: (d) => {
    const e = noErrors();
    requireName(e, d.name, 'a code');
    // The server's rule too (422 `invalid` on `name`, which lands here through `recordServerErrors`).
    if (d.name.trim().length > 3) e.header.name = 'A sales tax code is one to three characters (Tax, Non, Out…).';
    return e;
  },
  isEmpty: (d) => !d.name.trim() && !d.description,
  fingerprint: (d) => JSON.stringify(salesTaxCodeSpec.toRecord(d)),
  serverErrors: (_d, err) => recordServerErrors(err),
  nextDraft: () => blankSalesTaxCode(''),
  newPath: () => paths.newRecord('sales-tax-codes'),
  title: (_d, saved) => (saved ? `Edit Sales Tax Code: ${saved.name}` : 'New Sales Tax Code'),
  savedMessage: (r) => `Sales tax code "${r.name}" saved.`,
};

export function SalesTaxCodeEditor({ saved, params }: { saved: SalesTaxCode | null; params: Record<string, string> }) {
  const firstField = useRef<HTMLInputElement>(null);
  const form = useRecordForm({ spec: salesTaxCodeSpec, saved, params, onSavedNew: () => firstField.current?.focus() });
  const { draft: d, update, errors, mobile } = form;
  return (
    <Shell form={form} title={saved ? 'Edit Sales Tax Code' : 'New Sales Tax Code'} testId="record-sales-tax-code" inactiveLabel="Code is inactive" inactive={!d.is_active} onInactive={(v) => update({ is_active: !v })}>
      <SimpleGrid cols={{ base: 1, sm: 3 }}>
        <TextInput label="Sales tax code" description="1 to 3 characters" value={d.name} onChange={(e) => update({ name: e.currentTarget.value })} maxLength={3} required error={errors.header.name} ref={firstField} autoFocus={!mobile} aria-label="Sales tax code" />
        <TextInput label="Description" value={d.description ?? ''} onChange={(e) => update({ description: e.currentTarget.value })} style={{ gridColumn: mobile ? undefined : 'span 2' }} aria-label="Description" />
      </SimpleGrid>
      <SegmentedControl data={[{ value: 'taxable', label: 'Taxable' }, { value: 'non', label: 'Non-taxable' }]} value={d.is_taxable ? 'taxable' : 'non'} onChange={(v) => update({ is_taxable: v === 'taxable' })} />
    </Shell>
  );
}
