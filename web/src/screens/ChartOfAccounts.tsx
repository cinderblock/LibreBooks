// Chart of Accounts (Ctrl+A): the account tree with types and rolled-up
// balances. Enter or Ctrl+R opens a balance-sheet account's register; for
// income and expense accounts it opens the QuickReport (as QBD does). Ctrl+N
// new, Ctrl+E edit, Ctrl+D delete, Ctrl+Q QuickReport.

import { Alert, Button, Checkbox, Group, Modal, Select, Stack, Text, TextInput, Textarea } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconPlus, IconSearch } from '@tabler/icons-react';
import { useMemo, useState } from 'react';
import { describeError } from '~/api/errors';
import { useAccounts, useBalanceMap, useCompanyInfo, useDeleteRecord, useSaveRecord } from '~/api/hooks';
import type { Account, AccountType } from '~/api/types';
import { KeyList } from '~/components/KeyList';
import { useQuickCreate } from '~/inputs/QuickCreate';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { ACCOUNT_TYPES, ACCOUNT_TYPE_LABEL, hasRegister, isBalanceSheet, naturalBalance } from '~/lib/accountTypes';
import { formatMoney } from '~/lib/money';
import { prepare, search } from '~/lib/quickfill';
import { rollUp } from '~/lib/tree';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow, useOpenWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

export function openAccount(open: (p: string) => void, a: Account) {
  open(hasRegister(a.account_type) ? paths.register(a.id) : paths.report('AccountQuickReport', { acct: a.id, dates: 'ThisFiscalYearToDate' }));
}

export function ChartOfAccounts() {
  const win = useWindow();
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const { data: accounts = [], isLoading } = useAccounts();
  const { data: info } = useCompanyInfo();
  const balances = useBalanceMap();
  const qc = useQuickCreate();
  const del = useDeleteRecord('accounts');
  const [query, setQuery] = useState('');
  const [type, setType] = useState<string | null>(null);
  const [showInactive, setShowInactive] = useState(false);
  const [selected, setSelected] = useState(0);
  const [editing, setEditing] = useState<Account | null>(null);

  const totals = useMemo(() => rollUp(accounts, new Map(accounts.map((a) => [a.id, balances.get(a.id) ?? 0]))), [accounts, balances]);
  const prepared = useMemo(() => prepare(accounts.map((a) => ({ fullName: a.full_name, keys: a.number ? [a.number] : [] }))), [accounts]);
  const rows = useMemo(() => {
    const order = query.trim() ? search(prepared, query).sort((x, y) => x - y) : accounts.map((_, i) => i);
    return order.map((i) => accounts[i]!).filter((a) => (showInactive || a.is_active) && (!type || a.account_type === type));
  }, [accounts, prepared, query, type, showInactive]);
  const parents = useMemo(() => new Set(accounts.map((a) => a.parent_id).filter(Boolean)), [accounts]);
  const current = rows[selected];
  const numbers = info?.preferences.use_account_numbers ?? false;

  const enabled = win.isActiveNow;
  useShortcut('nav.useRegister', () => (current ? openAccount(open, current) : false), { enabled, scope: 'window' });
  useShortcut('report.quick', () => current && open(paths.report('AccountQuickReport', { acct: current.id, dates: 'ThisFiscalYearToDate' })), { enabled });
  useShortcut('record.new', () => void qc.createAccount(''), { enabled });
  useShortcut('record.edit', () => current && setEditing(current), { enabled });
  useShortcut('record.delete', () => current && void remove(current), { enabled });

  const remove = async (a: Account) => {
    if (!window.confirm(`Delete the account "${a.full_name}"?`)) return;
    try {
      await del.mutateAsync({ id: a.id, editSeq: a.edit_seq });
      notifications.show({ message: `Deleted ${a.full_name}.` });
    } catch (e) {
      notifications.show({ color: 'red', title: "Can't delete", message: describeError(e) });
    }
  };

  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="chart-of-accounts">
      <Group gap="xs" wrap={mobile ? 'wrap' : 'nowrap'} px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0}>
        <TextInput
          placeholder="Find an account"
          leftSection={<IconSearch size={14} />}
          value={query}
          onChange={(e) => {
            setQuery(e.currentTarget.value);
            setSelected(0);
          }}
          style={{ flex: mobile ? '1 1 100%' : '0 1 280px' }}
          aria-label="Find an account"
        />
        <Select
          placeholder="All types"
          data={ACCOUNT_TYPES.map((t) => ({ value: t, label: ACCOUNT_TYPE_LABEL[t] }))}
          value={type}
          onChange={(v) => {
            setType(v);
            setSelected(0);
          }}
          clearable
          style={{ flex: '0 1 220px' }}
          aria-label="Account type"
        />
        <Checkbox label="Include inactive" checked={showInactive} onChange={(e) => setShowInactive(e.currentTarget.checked)} />
        {!mobile && (
          <Button ml="auto" variant="light" leftSection={<IconPlus size={16} />} onClick={() => void qc.createAccount('')}>
            New account
          </Button>
        )}
      </Group>
      {isLoading ? (
        <Text c="dimmed" p="md">
          Loading accounts…
        </Text>
      ) : (
        <KeyList
          label="Chart of Accounts"
          items={rows}
          rowKey={(a) => a.id}
          selected={selected}
          onSelect={setSelected}
          onOpen={(a) => openAccount(open, a)}
          jumpText={(a) => a.name}
          mobile={mobile}
          autoFocus={!mobile}
          rowAttrs={(a) => ({ 'data-inactive': !a.is_active || undefined, 'data-parent': parents.has(a.id) || undefined })}
          columns={[
            {
              key: 'name',
              header: 'Name',
              width: 'minmax(220px, 3fr)',
              render: (a) => (
                <span style={{ paddingLeft: query ? 0 : a.sublevel * 18 }}>
                  {numbers && a.number ? `${a.number} · ` : ''}
                  {query ? a.full_name : a.name}
                  {!a.is_active ? ' (inactive)' : ''}
                </span>
              ),
            },
            { key: 'type', header: 'Type', width: 'minmax(140px, 1fr)', render: (a) => ACCOUNT_TYPE_LABEL[a.account_type] },
            {
              key: 'balance',
              header: 'Balance Total',
              width: '140px',
              align: 'right',
              render: (a) => (isBalanceSheet(a.account_type) ? formatMoney(naturalBalance(a.account_type, totals.get(a.id) ?? 0)) : ''),
            },
          ]}
          renderMobile={(a) => (
            <>
              <span style={{ paddingLeft: query ? 0 : a.sublevel * 14, minWidth: 0 }}>
                <Text size="sm" fw={parents.has(a.id) ? 600 : 400} truncate>
                  {a.name}
                </Text>
                <Text size="xs" c="dimmed">
                  {ACCOUNT_TYPE_LABEL[a.account_type]}
                </Text>
              </span>
              <Text size="sm" className={classes.num}>
                {isBalanceSheet(a.account_type) ? formatMoney(naturalBalance(a.account_type, totals.get(a.id) ?? 0)) : ''}
              </Text>
            </>
          )}
        />
      )}
      {editing && <EditAccountModal account={editing} accounts={accounts} onClose={() => setEditing(null)} />}
    </div>
  );
}

function EditAccountModal({ account, accounts, onClose }: { account: Account; accounts: Account[]; onClose(): void }) {
  const [name, setName] = useState(account.name);
  const [number, setNumber] = useState(account.number ?? '');
  const [description, setDescription] = useState(account.description ?? '');
  const [parentId, setParentId] = useState(account.parent_id);
  const [active, setActive] = useState(account.is_active);
  const [type, setType] = useState<AccountType>(account.account_type);
  const save = useSaveRecord('accounts');
  // A subaccount must be of its parent's type, and not inside itself.
  const descendants = new Set([account.id, ...accounts.filter((a) => a.full_name.startsWith(`${account.full_name}:`)).map((a) => a.id)]);
  const parents = accounts.filter((a) => a.account_type === type && !descendants.has(a.id));
  return (
    <Modal opened onClose={onClose} title={`Edit Account: ${account.full_name}`} centered>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate(
            { ...account, name: name.trim(), number: number.trim() || null, description: description.trim() || null, parent_id: parentId, is_active: active, account_type: type },
            { onSuccess: onClose },
          );
        }}
      >
        <Stack gap="sm">
          <TextInput label="Account name" value={name} onChange={(e) => setName(e.currentTarget.value)} required data-autofocus />
          <Select
            label="Type"
            data={ACCOUNT_TYPES.map((t) => ({ value: t, label: ACCOUNT_TYPE_LABEL[t] }))}
            value={type}
            onChange={(v) => {
              if (v) setType(v as AccountType);
              setParentId(null);
            }}
            allowDeselect={false}
            disabled={!!account.role}
            description={account.role ? 'This account has a special role in the books; its type is fixed.' : undefined}
          />
          <Select label="Subaccount of" placeholder="(top level)" data={parents.map((a) => ({ value: a.id, label: a.full_name }))} value={parentId} onChange={setParentId} clearable searchable />
          <TextInput label="Number" value={number} onChange={(e) => setNumber(e.currentTarget.value)} />
          <Textarea label="Description" value={description} onChange={(e) => setDescription(e.currentTarget.value)} autosize />
          <Checkbox label="Account is active" checked={active} onChange={(e) => setActive(e.currentTarget.checked)} />
          {save.error && <Alert color="red">{describeError(save.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" loading={save.isPending}>
              Save
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}
