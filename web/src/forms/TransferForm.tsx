// Transfer Funds: from one balance-sheet account to another, with both balances
// in view.

import { Group, Text, TextInput } from '@mantine/core';
import { useMemo, useRef } from 'react';
import { useAccountIndex, useBalanceMap, useCompanyInfo } from '~/api/hooks';
import type { SaveOutcome, Txn } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill } from '~/inputs/pickers';
import { useDateWarning } from '~/inputs/useDateWarning';
import { ACCOUNT_TYPES, hasRegister, naturalBalance } from '~/lib/accountTypes';
import { formatDateShort, todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { paths } from '~/windows/paths';
import type { TxnFormSpec } from './model';
import { isTransferEmpty, newTransferDraft, nextTransferDraft, transferDraftFromTxn, transferDraftToTxn, transferFingerprint, transferServerErrors, validateTransfer, type TransferDraft } from './transferDraft';
import { FormBadges, TxnFormActions, useTxnForm } from './useTxnForm';
import classes from './forms.module.css';

const REGISTER_TYPES = ACCOUNT_TYPES.filter(hasRegister);

function spec(defaultFrom: () => string | null): TxnFormSpec<TransferDraft> {
  return {
    noun: 'transfer',
    newDraft: () => newTransferDraft(todayIso(), defaultFrom()),
    fromTxn: transferDraftFromTxn,
    toTxn: transferDraftToTxn,
    validate: validateTransfer,
    isEmpty: isTransferEmpty,
    fingerprint: transferFingerprint,
    serverErrors: transferServerErrors,
    nextDraft: nextTransferDraft,
    newPath: paths.form('transfer'),
    savedMessage: (out: SaveOutcome) => `Transfer of ${formatMoney(out.total, { symbol: true })} saved.`,
    siblings: () => ({ types: ['Transfer'] }),
    title: (_d, saved) => (saved ? `Transfer ${formatDateShort(saved.date)}` : 'Transfer Funds'),
  };
}

export function TransferForm({ saved, initial }: { saved: Txn | null; initial: Txn | null }) {
  const { data: info } = useCompanyInfo();
  const accounts = useAccountIndex();
  const balances = useBalanceMap();
  const firstField = useRef<HTMLInputElement>(null);
  const form = useTxnForm({ spec: useMemo(() => spec(() => info?.preferences.default_check_account ?? null), [info]), saved, initial, onSavedNew: () => firstField.current?.focus() });
  const { draft, update, errors, mobile } = form;
  const dateWarning = useDateWarning(draft.date);
  const balanceOf = (id: string | null) => {
    const a = id ? accounts.get(id) : undefined;
    return a ? formatMoney(naturalBalance(a.account_type, balances.get(a.id) ?? 0)) : '—';
  };
  return (
    <div className={mobile ? classes.mobileForm : classes.form} data-testid="form-transfer">
      {form.banner}
      <Group align="flex-end" gap="md" wrap="wrap">
        <DateField label="Date" value={draft.date} onChange={(date) => date && update({ date })} error={errors.header.date} warning={dateWarning} aria-label="Date" />
        <FormBadges saved={saved} />
      </Group>
      <div className={mobile ? undefined : classes.headerGrid} style={mobile ? undefined : { gridTemplateColumns: '1fr 160px' }}>
        <AccountQuickFill
          label="Transfer Funds From"
          types={REGISTER_TYPES}
          value={draft.account_id}
          onChange={(account_id) => update({ account_id })}
          error={errors.header.account}
          inputRef={firstField}
          autoFocus={!saved && !mobile}
          allowAddNew={false}
          aria-label="Transfer from"
        />
        <Text size="sm" c="dimmed" pb={6}>
          Balance <Text span fw={600} c="var(--mantine-color-text)">{balanceOf(draft.account_id)}</Text>
        </Text>
        <AccountQuickFill label="Transfer Funds To" types={REGISTER_TYPES} value={draft.to_account_id} onChange={(to_account_id) => update({ to_account_id })} error={errors.header.to_account} allowAddNew={false} aria-label="Transfer to" />
        <Text size="sm" c="dimmed" pb={6}>
          Balance <Text span fw={600} c="var(--mantine-color-text)">{balanceOf(draft.to_account_id)}</Text>
        </Text>
        <MoneyField label="Transfer Amount" value={draft.amount} onChange={(amount) => update({ amount })} error={errors.header.amount} aria-label="Transfer amount" size={mobile ? 'md' : 'sm'} />
        <span />
        <TextInput label="Memo" value={draft.memo} onChange={(e) => { const memo = e.currentTarget.value; update({ memo }); }} className={classes.full} aria-label="Memo" />
      </div>
      {form.formErrors}
      <TxnFormActions form={form} />
      {form.chrome}
    </div>
  );
}
