// An account register window (Ctrl+R). Ctrl+G jumps to the register on the
// other side of the selected line; Ctrl+Q opens its QuickReport; Ctrl+N starts a
// new check (bank accounts).

import { Alert, Button, Group, Skeleton, Text } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconPlus } from '@tabler/icons-react';
import { useEffect, useRef } from 'react';
import { describeError, isApiError } from '~/api/errors';
import { useAccountIndex, useBalanceMap, useRegister, useSaveTxn, useSetCleared } from '~/api/hooks';
import type { RegisterLine } from '~/api/types';
import { registerEntryToTxn } from '~/forms/registerEntry';
import { RegisterGrid } from '~/inputs/RegisterGrid';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { hasRegister, naturalBalance } from '~/lib/accountTypes';
import { formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

export function Register() {
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

  useEffect(() => {
    if (account) win.setTitle(account.full_name);
  }, [account?.full_name]); // eslint-disable-line react-hooks/exhaustive-deps

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
  useShortcut('report.quick', () => {
    if (accountId) open(paths.report('account_quickreport', { account: accountId, macro: 'ThisFiscalYearToDate' }));
  }, { enabled });
  useShortcut('record.new', () => (account?.account_type === 'Bank' ? open(paths.newCheck()) : false), { enabled });
  useShortcut('record.edit', () => (selection.current ? open(paths.txn(selection.current.txn_id)) : false), { enabled });

  if (!accountId) return <Alert m="md">No account chosen.</Alert>;
  if (reg.error) return <Alert m="md" color="red">{describeError(reg.error)}</Alert>;
  if (!account || reg.isLoading || !reg.data) return <Skeleton m="md" h={400} />;

  const balance = naturalBalance(account.account_type, balances.get(account.id) ?? 0);
  const toggle = (l: RegisterLine) => cleared.mutate({ txnId: l.txn_id, status: l.cleared === 'Cleared' ? 'Uncleared' : 'Cleared' });
  const openLine = (l: RegisterLine) => {
    if (l.txn_id.startsWith('stress-')) {
      notifications.show({ message: 'Stress-test lines are synthetic and have no transaction behind them.' });
      return;
    }
    open(paths.txn(l.txn_id));
  };
  const canEnter = !['AccountsReceivable', 'AccountsPayable'].includes(account.account_type) && !mobile;

  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="register">
      <Group justify="space-between" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0}>
        <Text size="sm" c="dimmed">
          {reg.data.lines.length.toLocaleString()} transactions
        </Text>
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
        onOpen={openLine}
        onToggleCleared={toggle}
        onSelectionChange={(l) => (selection.current = l)}
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
    </div>
  );
}
