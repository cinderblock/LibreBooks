// Make General Journal Entries: debit and credit columns with a running
// difference; leaving a line prefills the next with what balances the entry
// (QBD's behaviour), and the entry must balance to save.

import { Checkbox, Group, Text, TextInput } from '@mantine/core';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useCompanyInfo, useNextNumber } from '~/api/hooks';
import type { SaveOutcome, Txn } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { LineGrid, type GridColumn } from '~/inputs/LineGrid';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, ClassQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useDateWarning } from '~/inputs/useDateWarning';
import { todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { paths } from '~/windows/paths';
import { gridErrors, type TxnFormSpec } from './model';
import {
  blankJournalRow,
  credits,
  debits,
  difference,
  isBlankJournal,
  isJournalEmpty,
  journalDraftFromTxn,
  journalDraftToTxn,
  journalFingerprint,
  journalServerErrors,
  newJournalDraft,
  nextJournalDraft,
  setCredit,
  setDebit,
  validateJournal,
  withBalancingLine,
  type JournalDraft,
  type JournalRow,
} from './journalDraft';
import { FormBadges, TxnFormActions, useTxnForm } from './useTxnForm';
import classes from './forms.module.css';

function spec(autoNumber: () => boolean): TxnFormSpec<JournalDraft> {
  return {
    noun: 'journal entry',
    newDraft: () => newJournalDraft(todayIso(), autoNumber()),
    fromTxn: journalDraftFromTxn,
    toTxn: journalDraftToTxn,
    validate: validateJournal,
    isEmpty: isJournalEmpty,
    fingerprint: journalFingerprint,
    serverErrors: journalServerErrors,
    nextDraft: nextJournalDraft,
    newPath: paths.form('journal'),
    savedMessage: (out: SaveOutcome) => `Journal entry ${out.txn.number ?? ''} for ${formatMoney(out.total, { symbol: true })} saved.`.replace('  ', ' '),
    siblings: () => ({ types: ['JournalEntry'] }),
    title: (_d, saved) => (saved ? `General Journal ${saved.number ?? ''}`.trim() : 'Make General Journal Entries'),
  };
}

export function JournalForm({ saved, initial }: { saved: Txn | null; initial: Txn | null }) {
  const { data: info } = useCompanyInfo();
  const firstField = useRef<HTMLInputElement>(null);
  const form = useTxnForm({ spec: useMemo(() => spec(() => info?.preferences.auto_number_journal_entries ?? true), [info]), saved, initial, onSavedNew: () => firstField.current?.focus() });
  const { draft, update, errors, mobile, enterBehavior } = form;
  const wantsAuto = draft.edit_seq === 0 && draft.numberIsAuto;
  const next = useNextNumber('JournalEntry', null, wantsAuto);
  useEffect(() => {
    if (next.data !== undefined && wantsAuto) update((d) => (d.numberIsAuto ? { ...d, number: next.data ?? '' } : d));
  }, [next.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const dateWarning = useDateWarning(draft.date);
  const useClasses = info?.preferences.use_class_tracking ?? false;
  const onEnterSave = useCallback(() => void form.save('close'), []); // eslint-disable-line react-hooks/exhaustive-deps
  const diff = difference(draft.rows);

  const columns = useMemo<GridColumn<JournalRow>[]>(
    () => [
      {
        key: 'account',
        header: 'Account',
        width: '26%',
        render: (c) => (
          <AccountQuickFill variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" value={c.row.account_id} onChange={(id) => c.update((r) => ({ ...r, account_id: id }))} onCommit={c.onCommit} error={c.error} inputRef={c.rowIndex === 0 ? firstField : undefined} inputProps={c.cellProps} />
        ),
      },
      {
        key: 'debit',
        header: 'Debit',
        width: 110,
        align: 'right',
        mobile: 'half',
        render: (c) => <MoneyField variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" blankZero value={c.row.debit} onChange={(v) => c.update((r) => setDebit(r, v))} error={c.error} inputProps={c.cellProps} />,
        footer: <span className={classes.total}>{formatMoney(debits(draft.rows))}</span>,
      },
      {
        key: 'credit',
        header: 'Credit',
        width: 110,
        align: 'right',
        mobile: 'half',
        render: (c) => <MoneyField variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" blankZero value={c.row.credit} onChange={(v) => c.update((r) => setCredit(r, v))} inputProps={c.cellProps} />,
        footer: <span className={classes.total}>{formatMoney(credits(draft.rows))}</span>,
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
        key: 'name',
        header: 'Name',
        width: '16%',
        mobile: 'full',
        render: (c) => <NameQuickFill variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" value={c.row.entity_id} onChange={(id) => c.update((r) => ({ ...r, entity_id: id }))} onCommit={c.onCommit} error={c.error} inputProps={c.cellProps} />,
      },
      ...(useClasses
        ? [
            {
              key: 'class',
              header: 'Class',
              width: '12%',
              mobile: 'half' as const,
              render: (c) => <ClassQuickFill variant={c.variant === 'cell' ? 'cell' : 'default'} label={c.variant === 'default' ? c.label : undefined} size="xs" value={c.row.class_id} onChange={(id) => c.update((r) => ({ ...r, class_id: id }))} onCommit={c.onCommit} inputProps={c.cellProps} />,
            } satisfies GridColumn<JournalRow>,
          ]
        : []),
    ],
    [useClasses, draft.rows], // eslint-disable-line react-hooks/exhaustive-deps
  );

  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid="form-journal">
      {form.banner}
      <Group align="flex-end" gap="md" wrap="wrap">
        <DateField label="Date" value={draft.date} onChange={(date) => date && update({ date })} error={errors.header.date} warning={dateWarning} aria-label="Date" />
        <TextInput label="Entry No." value={draft.number} onChange={(e) => { const number = e.currentTarget.value; update((d) => ({ ...d, number, numberIsAuto: false })); }} error={errors.header.number} w={110} aria-label="Entry number" />
        <Checkbox label="Adjusting entry" checked={draft.adjusting} onChange={(e) => update({ adjusting: e.currentTarget.checked })} mb={6} />
        <FormBadges saved={saved} />
        <Text size="sm" ml="auto" className={diff === 0 ? classes.balanced : classes.unbalanced} data-testid="journal-difference">
          {diff === 0 ? 'Balanced' : `Difference ${formatMoney(Math.abs(diff))} (${diff > 0 ? 'credit needed' : 'debit needed'})`}
        </Text>
      </Group>
      {form.formErrors}
      <div onBlur={() => update((d) => ({ ...d, rows: withBalancingLine(d.rows) }))}>
        <LineGrid label="Journal lines" rows={draft.rows} onChange={(rows) => update({ rows })} columns={columns} rowKey={(r) => r.key} isBlank={isBlankJournal} makeBlank={blankJournalRow} errors={gridErrors(errors, 'rows')} enterBehavior={enterBehavior} onEnterSave={onEnterSave} mobile={mobile} minVisibleRows={6} />
      </div>
      <TxnFormActions form={form} />
      {form.chrome}
    </div>
  );
}
