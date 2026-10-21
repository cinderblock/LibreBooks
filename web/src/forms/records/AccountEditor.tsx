// New Account / Edit Account (Chart of Accounts: Ctrl+N, Ctrl+E). QBD's dialog:
// the type, the name and number, "Subaccount of", description, the bank's
// account number, the tax-line mapping, and for a new balance-sheet account an
// opening balance as of a date. The opening balance is what QBD does under the
// hood too: a general journal entry against Opening Balance Equity, which the
// server posts in the same write as the account (`opening_balance` + `as_of`
// on `POST /accounts`), so the account never lands without its balance.

import { Checkbox, Group, Select, SimpleGrid, Text, Textarea, TextInput, Title } from '@mantine/core';
import { useMemo, useRef } from 'react';
import { blankAccount } from '~/api/blank';
import { useAccounts, useBalanceMap, useCompanyInfo } from '~/api/hooks';
import type { Account, AccountType, CashFlowClass } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { accountOptions } from '~/inputs/pickers';
import { QuickFill } from '~/inputs/QuickFill';
import { ACCOUNT_TYPES, ACCOUNT_TYPE_LABEL, isBalanceSheet } from '~/lib/accountTypes';
import { todayIso, type IsoDate } from '~/lib/dates';
import type { Cents } from '~/lib/money';
import { paths } from '~/windows/paths';
import { noErrors, type FormErrors } from '../model';
import { RecordFormActions, useRecordForm, type RecordFormSpec } from '../useRecordForm';
import classes from '../forms.module.css';
import { fullNameOf, possibleParents, recordServerErrors, requireName, textOrNull } from './model';

export interface AccountDraft {
  rec: Account;
  /** A new balance-sheet account's opening balance (natural sign), sent with the create as `opening_balance`. */
  opening: Cents | null;
  openingDate: IsoDate;
}

const CASH_FLOW: Array<{ value: CashFlowClass; label: string }> = [
  { value: 'Operating', label: 'Operating' },
  { value: 'Investing', label: 'Investing' },
  { value: 'Financing', label: 'Financing' },
  { value: 'NotApplicable', label: 'Not applicable' },
];

const isAccountType = (t: string | undefined): t is AccountType => !!t && (ACCOUNT_TYPES as readonly string[]).includes(t);

/** Opening balances: balance-sheet accounts QBD lets you open with a figure (not A/R, A/P or their special roles). */
function takesOpeningBalance(t: AccountType): boolean {
  return isBalanceSheet(t) && t !== 'AccountsReceivable' && t !== 'AccountsPayable';
}

export function validateAccount(d: AccountDraft): FormErrors {
  const e = noErrors();
  requireName(e, d.rec.name, 'an account name');
  if (d.opening && !d.openingDate) e.header.openingDate = 'Enter the date the balance is as of.';
  return e;
}

const spec: RecordFormSpec<'accounts', AccountDraft> = {
  list: 'accounts',
  noun: 'account',
  newDraft: (params) => ({ rec: blankAccount('', isAccountType(params.type) ? params.type : 'Expense'), opening: null, openingDate: todayIso() }),
  fromRecord: (rec) => ({ rec, opening: null, openingDate: todayIso() }),
  toRecord: (d) => ({ ...d.rec, name: d.rec.name.trim(), number: textOrNull(d.rec.number), description: textOrNull(d.rec.description), bank_account_number: textOrNull(d.rec.bank_account_number), tax_line: textOrNull(d.rec.tax_line) }),
  // Natural sign as entered: the server debits a bank balance and credits a loan's, Opening Balance Equity taking the
  // other side. A balance typed before the type changed to one without the field (the field hides) is not sent.
  opening: (d) => (d.opening && takesOpeningBalance(d.rec.account_type) ? { opening_balance: d.opening, as_of: d.openingDate } : null),
  validate: validateAccount,
  isEmpty: (d) => !d.rec.name.trim() && !d.rec.number && !d.rec.description && d.opening === null,
  fingerprint: (d) => JSON.stringify([spec.toRecord(d), d.opening, d.openingDate]),
  serverErrors: (_d, err) => recordServerErrors(err, { parent_id: 'parent', account_type: 'type' }),
  // Save & New keeps the type (QBD's dialog does), nothing else.
  nextDraft: (d) => ({ rec: blankAccount('', d.rec.account_type), opening: null, openingDate: d.openingDate }),
  newPath: (d) => paths.newRecord('accounts', { type: d.rec.account_type }),
  title: (_d, saved) => (saved ? `Edit Account: ${saved.full_name}` : 'New Account'),
  savedMessage: (r) => `Account "${r.full_name}" saved.`,
  attachmentKind: 'account',
};

export function AccountEditor({ saved, params }: { saved: Account | null; params: Record<string, string> }) {
  const { data: accounts = [] } = useAccounts();
  const balances = useBalanceMap();
  const { data: info } = useCompanyInfo();
  const firstField = useRef<HTMLInputElement>(null);
  const form = useRecordForm({ spec, saved, params, onSavedNew: () => firstField.current?.focus() });
  const { draft, update, errors, mobile } = form;
  const rec = draft.rec;
  const type = rec.account_type;
  const typeFixed = !!saved?.role;
  const setRec = (patch: Partial<Account>) => update((d) => ({ ...d, rec: { ...d.rec, ...patch } }));

  // A subaccount is of its parent's type, and never inside itself.
  const parentOptions = useMemo(() => accountOptions(possibleParents(accounts.filter((a) => a.account_type === type), saved), balances), [accounts, balances, type, saved]);
  const parent = rec.parent_id ? accounts.find((a) => a.id === rec.parent_id) : undefined;
  const numbers = info?.preferences.use_account_numbers ?? false;
  const bankLike = type === 'Bank' || type === 'CreditCard';
  const opening = !saved && takesOpeningBalance(type);

  return (
    <div className={mobile ? classes.mobileForm : classes.form} style={mobile ? undefined : { maxWidth: 720 }} data-testid="record-account" {...form.enterProps}>
      {form.banner}
      <Group justify="space-between" align="flex-start">
        <Title order={4}>{saved ? 'Edit Account' : 'New Account'}</Title>
        <Checkbox label="Account is inactive" checked={!rec.is_active} onChange={(e) => setRec({ is_active: !e.currentTarget.checked })} />
      </Group>
      <SimpleGrid cols={{ base: 1, sm: 2 }}>
        <Select
          label="Account type"
          data={ACCOUNT_TYPES.map((t) => ({ value: t, label: ACCOUNT_TYPE_LABEL[t] }))}
          value={type}
          onChange={(v) => {
            if (!isAccountType(v ?? undefined)) return;
            setRec({ account_type: v as AccountType, parent_id: null, sublevel: 0, full_name: rec.name });
          }}
          allowDeselect={false}
          disabled={typeFixed}
          description={typeFixed ? 'This account has a special role in the books; its type is fixed.' : undefined}
          error={errors.header.type}
          aria-label="Account type"
        />
        <TextInput label={numbers ? 'Number' : 'Number (numbering is off in Preferences)'} value={rec.number ?? ''} onChange={(e) => setRec({ number: e.currentTarget.value })} error={errors.header.number} aria-label="Account number" />
      </SimpleGrid>
      <TextInput
        label="Account name"
        value={rec.name}
        onChange={(e) => {
          const name = e.currentTarget.value;
          update((d) => ({ ...d, rec: { ...d.rec, name, full_name: fullNameOf(name, parent) } }));
        }}
        required
        error={errors.header.name}
        ref={firstField}
        autoFocus={!mobile}
        aria-label="Account name"
      />
      <QuickFill
        label="Subaccount of"
        placeholder="(top level)"
        options={parentOptions}
        columnHeaders={['Type', 'Balance']}
        value={rec.parent_id}
        onChange={(id) => {
          const p = id ? accounts.find((a) => a.id === id) : undefined;
          setRec({ parent_id: p?.id ?? null, sublevel: p ? p.sublevel + 1 : 0, full_name: fullNameOf(rec.name, p) });
        }}
        error={errors.header.parent}
        aria-label="Subaccount of"
      />
      <Textarea label="Description" value={rec.description ?? ''} onChange={(e) => setRec({ description: e.currentTarget.value })} autosize minRows={1} aria-label="Description" />
      <SimpleGrid cols={{ base: 1, sm: 2 }}>
        {bankLike && <TextInput label={type === 'Bank' ? 'Bank account number' : 'Card number (last digits)'} value={rec.bank_account_number ?? ''} onChange={(e) => setRec({ bank_account_number: e.currentTarget.value })} aria-label="Bank account number" />}
        <TextInput label="Tax-line mapping" placeholder="B/S-Assets: Cash" value={rec.tax_line ?? ''} onChange={(e) => setRec({ tax_line: e.currentTarget.value })} aria-label="Tax-line mapping" />
        {isBalanceSheet(type) && <Select label="Cash flow classification" data={CASH_FLOW} value={rec.cash_flow_class} onChange={(v) => setRec({ cash_flow_class: (v as CashFlowClass | null) ?? null })} clearable aria-label="Cash flow classification" />}
      </SimpleGrid>
      {opening && (
        <div className={mobile ? undefined : classes.headerGrid} style={mobile ? undefined : { gridTemplateColumns: '200px 180px 1fr' }}>
          <MoneyField label="Opening balance" value={draft.opening} onChange={(opening) => update({ opening })} blankZero aria-label="Opening balance" />
          <DateField label="as of" value={draft.openingDate} onChange={(d) => d && update({ openingDate: d })} error={errors.header.openingDate} aria-label="Opening balance as of" />
          <Text size="xs" c="dimmed" style={{ alignSelf: 'end' }}>
            Saved with the account as a journal entry against Opening Balance Equity. Leave blank to enter history as transactions.
          </Text>
        </div>
      )}
      {form.formErrors}
      <RecordFormActions form={form} />
      {form.chrome}
    </div>
  );
}

