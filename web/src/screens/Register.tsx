// An account register window (Ctrl+R). Ctrl+E edits the selected line in place
// (simple transactions; anything split opens its form), Ctrl+Q / Enter /
// double-click QuickZooms into the transaction's form, Ctrl+G jumps to the
// register on the other side, Ctrl+N starts a new check (bank accounts). The
// "1-Line" switch shows one row per transaction.

import { Alert, Button, Checkbox, Group, Skeleton, Text } from '@mantine/core';
import { useLocalStorage } from '@mantine/hooks';
import { notifications } from '@mantine/notifications';
import { useQueryClient } from '@tanstack/react-query';
import { IconPlus, IconReportAnalytics } from '@tabler/icons-react';
import { useEffect, useRef, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import { useAccountIndex, useBalanceMap, useRegister, useSaveTxn, useSetCleared } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { RegisterLine, Txn } from '~/api/types';
import { ConflictModal } from '~/forms/common';
import { applyEntryToTxn, registerEntryFromTxn, registerEntryToTxn, type RegisterEntry } from '~/forms/registerEntry';
import { RegisterGrid, type EditState } from '~/inputs/RegisterGrid';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { hasRegister, naturalBalance } from '~/lib/accountTypes';
import { formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

export function Register() {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  const win = useWindow();
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const accountId = win.desc.params.accountId ?? null;
  const accounts = useAccountIndex();
  const account = accountId ? accounts.get(accountId) : undefined;
  const reg = useRegister(accountId);
  const balances = useBalanceMap();
  const save = useSaveTxn();
  const cleared = useSetCleared(accountId ?? '');
  const selection = useRef<RegisterLine | null>(null);
  const [oneLine, setOneLine] = useLocalStorage({ key: 'librebooks.register.oneLine', defaultValue: false, getInitialValueInEffect: false });
  const [editing, setEditing] = useState<(EditState & { txn: Txn }) | null>(null);
  const [conflict, setConflict] = useState<Txn | null>(null);

  useEffect(() => {
    if (account) win.setTitle(account.full_name);
  }, [account?.full_name]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => win.setDirty(!!editing), [editing]); // eslint-disable-line react-hooks/exhaustive-deps

  const openLine = (l: RegisterLine) => {
    if (l.txn_id.startsWith('stress-')) {
      notifications.show({ message: 'Stress-test lines are synthetic and have no transaction behind them.' });
      return;
    }
    open(paths.txn(l.txn_id));
  };

  /** Ctrl+E: edit the selected line here when it's simple enough; else its form. */
  const editLine = async (l: RegisterLine) => {
    if (!account || l.txn_id.startsWith('stress-')) return openLine(l);
    try {
      const { txn } = await api.txn(co, l.txn_id);
      const entry = registerEntryFromTxn(account, txn);
      if (!entry) {
        notifications.show({ message: l.is_split ? 'This transaction has several lines; edit it in its form.' : 'This transaction is edited in its form.', color: 'gray' });
        return openLine(l);
      }
      setEditing({ txnId: txn.id, entry, txn });
    } catch (e) {
      notifications.show({ color: 'red', message: describeError(e) });
    }
  };

  const saveEdit = async (entry: RegisterEntry, overwrite = false): Promise<{ field: string; message: string } | null> => {
    if (!account || !editing) return null;
    let base = editing.txn;
    if (overwrite) base = { ...base, edit_seq: (await api.txn(co, base.id)).txn.edit_seq };
    const r = applyEntryToTxn(account, entry, base);
    if (!r.ok) return { field: r.field, message: r.message };
    try {
      await save.mutateAsync(r.txn);
      notifications.show({ message: 'Saved.', color: 'teal' });
      setEditing(null);
      return null;
    } catch (e) {
      if (isApiError(e) && e.isStaleConflict) {
        setConflict(r.txn);
        return null;
      }
      const msg = describeError(e);
      return { field: isApiError(e) && e.postingError?.code === 'MissingEntity' ? 'payee' : 'form', message: msg };
    }
  };

  const enabled = win.isActiveNow;
  useShortcut(
    'register.goToOther',
    () => {
      const l = selection.current;
      const other = l?.other_account_id ? accounts.get(l.other_account_id) : undefined;
      if (!other || !hasRegister(other.account_type)) {
        notifications.show({ message: l?.is_split ? 'This transaction has several accounts; open it to see them.' : 'The other account has no register.' });
        return;
      }
      open(paths.register(other.id));
    },
    { enabled },
  );
  // QuickZoom: the selected line's form (QBD's Ctrl+Q in a register).
  useShortcut('report.quick', () => (selection.current ? openLine(selection.current) : false), { enabled });
  useShortcut('record.new', () => (account?.account_type === 'Bank' ? open(paths.newCheck()) : false), { enabled });
  useShortcut('record.edit', () => (selection.current ? void editLine(selection.current) : false), { enabled });

  if (!accountId) return <Alert m="md">No account chosen.</Alert>;
  if (reg.error) return <Alert m="md" color="red">{describeError(reg.error)}</Alert>;
  if (!account || reg.isLoading || !reg.data) return <Skeleton m="md" h={400} />;

  const balance = naturalBalance(account.account_type, balances.get(account.id) ?? 0);
  const toggle = (l: RegisterLine) => cleared.mutate({ txnId: l.txn_id, status: l.cleared === 'Cleared' ? 'Uncleared' : 'Cleared' });
  const canEnter = !['AccountsReceivable', 'AccountsPayable'].includes(account.account_type) && !mobile;

  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="register">
      <Group justify="space-between" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0} wrap="wrap">
        <Group gap="sm">
          <Text size="sm" c="dimmed">
            {reg.data.lines.length.toLocaleString()} transactions
          </Text>
          {!mobile && <Checkbox size="xs" label="1-Line" checked={oneLine} onChange={(e) => setOneLine(e.currentTarget.checked)} />}
          {!mobile && (
            <Button size="compact-xs" variant="subtle" leftSection={<IconReportAnalytics size={14} />} onClick={() => open(paths.report('AccountQuickReport', { acct: account.id, dates: 'ThisFiscalYearToDate' }))}>
              QuickReport
            </Button>
          )}
          {editing && (
            <Text size="xs" c="blue">
              Editing a line: Enter saves, Esc cancels.
            </Text>
          )}
        </Group>
        <Group gap="xs">
          {mobile && account.account_type === 'Bank' && (
            <Button size="xs" leftSection={<IconPlus size={14} />} onClick={() => open(paths.newCheck())}>
              Check
            </Button>
          )}
          <Text size="sm">
            Ending balance{' '}
            <Text span fw={700} className={classes.num}>
              {formatMoney(balance)}
            </Text>
          </Text>
        </Group>
      </Group>
      <RegisterGrid
        account={account}
        lines={reg.data.lines}
        mobile={mobile}
        oneLine={oneLine}
        onOpen={openLine}
        onToggleCleared={toggle}
        onSelectionChange={(l) => (selection.current = l)}
        editing={editing}
        onSaveEdit={(entry) => saveEdit(entry)}
        onCancelEdit={() => setEditing(null)}
        onRecord={
          canEnter
            ? async (entry) => {
                const r = registerEntryToTxn(account, entry);
                if (!r.ok) return { field: r.field, message: r.message };
                try {
                  await save.mutateAsync(r.txn);
                  return null;
                } catch (e) {
                  const msg = describeError(e);
                  if (isApiError(e) && e.postingError?.code === 'MissingEntity') return { field: 'payee', message: msg };
                  return { field: 'form', message: msg };
                }
              }
            : undefined
        }
      />
      <ConflictModal
        opened={!!conflict}
        what="transaction"
        onCancel={() => setConflict(null)}
        onReload={() => {
          setConflict(null);
          setEditing(null);
          void qc.invalidateQueries({ queryKey: qk.txn(co, conflict!.id) });
          void qc.invalidateQueries({ queryKey: qk.registerAll(co) });
        }}
        onOverwrite={() => {
          const txn = conflict!;
          setConflict(null);
          void saveEdit(registerEntryFromTxn(account, txn) ?? editing!.entry, true);
        }}
      />
    </div>
  );
}
