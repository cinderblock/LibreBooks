// Customer, Vendor and Employee Centers: the names on the left (customers with
// their jobs underneath, Balance and Balance Total alongside as QBD shows
// them, from `GET /balances/names`), the selected name's details and
// transactions on the right. On a phone: the list, then the detail as its own
// screen with a back arrow.

import { ActionIcon, Alert, Badge, Button, Card, Checkbox, Group, Menu, SimpleGrid, Skeleton, Stack, Table, Text, TextInput, Title } from '@mantine/core';
import { IconChevronDown, IconPencil, IconPlus, IconSearch } from '@tabler/icons-react';
import { useEffect, useMemo, useState } from 'react';
import { describeError } from '~/api/errors';
import { useEntities, useNameBalanceMap, useTxns } from '~/api/hooks';
import type { Entity, EntityKind, NameBalance, TxnType } from '~/api/types';
import { KeyList } from '~/components/KeyList';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { prepare, search } from '~/lib/quickfill';
import { TXN_LABEL } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

interface CenterConfig {
  kind: EntityKind;
  title: string;
  /** Transactions that show an Open column in the detail (A/R or A/P items). */
  openTypes: TxnType[];
  /** Which side of `NameBalance` this centre's Balance is, or none (employees). */
  balance: 'receivable' | 'payable' | null;
  newTxns: Array<{ label: string; path: string }>;
}

/** A name's own balance and the one with its jobs rolled in, positive = owed. */
interface Balances {
  own: number;
  total: number;
}

const NOTHING_OPEN: Balances = { own: 0, total: 0 };

function balancesOf(nb: NameBalance | undefined, side: CenterConfig['balance']): Balances {
  if (!nb || !side) return NOTHING_OPEN;
  return side === 'receivable' ? { own: nb.receivable, total: nb.receivable_total } : { own: nb.payable, total: nb.payable_total };
}

const CONFIGS: Record<'customers' | 'vendors' | 'employees', CenterConfig> = {
  customers: {
    kind: 'Customer',
    title: 'Customer Center',
    openTypes: ['Invoice', 'StatementCharge', 'CreditMemo'],
    balance: 'receivable',
    newTxns: [
      { label: 'Create Invoice', path: paths.form('invoice') },
      { label: 'Receive Payment', path: paths.form('receive-payment') },
      { label: 'Sales Receipt', path: paths.form('sales-receipt') },
      { label: 'Estimate', path: paths.form('estimate') },
    ],
  },
  vendors: {
    kind: 'Vendor',
    title: 'Vendor Center',
    openTypes: ['Bill', 'VendorCredit'],
    balance: 'payable',
    newTxns: [
      { label: 'Write Check', path: paths.newCheck() },
      { label: 'Enter Bill', path: paths.form('bill') },
      { label: 'Pay Bills', path: paths.form('pay-bills') },
      { label: 'Purchase Order', path: paths.form('purchase-order') },
    ],
  },
  employees: {
    kind: 'Employee',
    title: 'Employee Center',
    openTypes: [],
    balance: null,
    newTxns: [{ label: 'Write Check', path: paths.newCheck() }],
  },
};

export function Center({ which }: { which: 'customers' | 'vendors' | 'employees' }) {
  const cfg = CONFIGS[which];
  const win = useWindow();
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const { data: all = [], isLoading } = useEntities();
  const [query, setQuery] = useState('');
  const [showInactive, setShowInactive] = useState(false);
  const [selected, setSelected] = useState(0);
  const [mobileDetail, setMobileDetail] = useState(false);

  const names = useMemo(() => all.filter((e) => e.kind === cfg.kind), [all, cfg.kind]);
  const prepared = useMemo(() => prepare(names.map((e) => ({ fullName: e.full_name, keys: e.company_name ? [e.company_name] : [] }))), [names]);
  const rows = useMemo(() => {
    const order = query.trim() ? search(prepared, query).sort((a, b) => a - b) : names.map((_, i) => i);
    return order.map((i) => names[i]!).filter((e) => showInactive || e.is_active);
  }, [names, prepared, query, showInactive]);

  // Balance (the name's own open items) and Balance Total (with its jobs), from the server.
  const nameBalances = useNameBalanceMap(cfg.balance !== null);
  const balances = (id: string): Balances => balancesOf(nameBalances.get(id), cfg.balance);

  // `?select=<id>` (from the palette or a report) picks a name.
  const selectParam = win.desc.params.select;
  useEffect(() => {
    if (!selectParam) return;
    const i = rows.findIndex((e) => e.id === selectParam);
    if (i >= 0) {
      setSelected(i);
      if (mobile) setMobileDetail(true);
    }
  }, [selectParam, rows.length]); // eslint-disable-line react-hooks/exhaustive-deps

  const current = rows[selected];
  const enabled = win.isActiveNow;
  // On a phone the detail is a sub-view: the header's Back returns to the list.
  const showingDetail = mobile && mobileDetail && !!current;
  useEffect(() => {
    win.setBackHandler(showingDetail ? () => setMobileDetail(false) : null);
    return () => win.setBackHandler(null);
  }, [showingDetail]); // eslint-disable-line react-hooks/exhaustive-deps
  // New and Edit open the name's editor window (QBD's New/Edit Customer dialog).
  const create = (parent?: string) => open(paths.newRecord('entities', { kind: cfg.kind, parent }));
  const edit = (e: Entity) => open(paths.record('entities', e.id));
  useShortcut('record.new', () => create(), { enabled });
  useShortcut('record.edit', () => (current ? edit(current) : false), { enabled });
  useShortcut('report.quick', () => current && open(paths.report('TransactionListByDate', { name: current.id, dates: 'All' })), { enabled });

  const list = (
    <div className={classes.centerList} style={mobile ? { width: '100%', flex: 1 } : undefined}>
      <Group gap="xs" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0} wrap="nowrap">
        <TextInput
          placeholder={`Find a ${cfg.kind.toLowerCase()}`}
          leftSection={<IconSearch size={14} />}
          value={query}
          onChange={(e) => {
            setQuery(e.currentTarget.value);
            setSelected(0);
          }}
          style={{ flex: 1 }}
          aria-label={`Find a ${cfg.kind.toLowerCase()}`}
        />
        <ActionIcon variant="light" size="lg" aria-label={`New ${cfg.kind}`} onClick={() => create()}>
          <IconPlus size={18} />
        </ActionIcon>
      </Group>
      <Checkbox px={mobile ? 'sm' : 0} size="xs" label="Include inactive" checked={showInactive} onChange={(e) => setShowInactive(e.currentTarget.checked)} />
      {isLoading ? (
        <Skeleton h={300} />
      ) : (
        <KeyList
          label={`${cfg.kind} list`}
          items={rows}
          rowKey={(e) => e.id}
          selected={selected}
          onSelect={setSelected}
          onOpen={() => mobile && setMobileDetail(true)}
          jumpText={(e) => e.name}
          mobile={mobile}
          autoFocus={!mobile}
          rowAttrs={(e) => ({ 'data-inactive': !e.is_active || undefined })}
          columns={[
            { key: 'name', header: 'Name', width: 'minmax(160px, 1fr)', render: (e) => <span style={{ paddingLeft: query ? 0 : e.sublevel * 16 }}>{query ? e.full_name : e.name}</span> },
            ...(cfg.balance
              ? [
                  { key: 'bal', header: 'Balance', width: '110px', align: 'right' as const, render: (e: Entity) => formatMoney(balances(e.id).own, { blankZero: true }) },
                  { key: 'total', header: 'Balance Total', width: '110px', align: 'right' as const, render: (e: Entity) => formatMoney(balances(e.id).total, { blankZero: true }) },
                ]
              : []),
          ]}
          renderMobile={(e) => (
            <>
              <span style={{ paddingLeft: query ? 0 : e.sublevel * 14, minWidth: 0 }}>
                <Text size="sm" fw={e.sublevel ? 400 : 600} truncate>
                  {e.name}
                </Text>
                {e.company_name && (
                  <Text size="xs" c="dimmed" truncate>
                    {e.company_name}
                  </Text>
                )}
              </span>
              <Text size="sm" className={classes.num}>
                {formatMoney(balances(e.id).total, { blankZero: true })}
              </Text>
            </>
          )}
        />
      )}
    </div>
  );

  const detail = current ? (
    <NameDetail entity={current} balance={balances(current.id)} cfg={cfg} onEdit={() => edit(current)} onAddJob={cfg.kind === 'Customer' ? () => create(current.id) : undefined} phone={mobile} />
  ) : (
    <Text c="dimmed" p="md">
      Choose a name.
    </Text>
  );

  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid={`center-${which}`}>
      {mobile ? (mobileDetail && current ? detail : list) : (
        <div className={classes.center}>
          {list}
          <div className={classes.centerDetail}>{detail}</div>
        </div>
      )}
    </div>
  );
}

function NameDetail({ entity, balance, cfg, onEdit, onAddJob, phone }: { entity: Entity; balance: Balances; cfg: CenterConfig; onEdit(): void; onAddJob?(): void; phone: boolean }) {
  const open = useOpenWindow();
  const txns = useTxns({ entity_id: entity.id, include_jobs: true, descending: true, limit: 200 });
  const addr = entity.bill_address;
  const address = [...(addr.lines ?? []), [addr.city, addr.state, addr.postal_code].filter(Boolean).join(' ')].filter(Boolean);
  return (
    <Stack gap="sm" p={phone ? 'sm' : 0}>
      <Group justify="space-between" wrap={phone ? 'wrap' : 'nowrap'} align="flex-start" gap="xs">
        <Group gap="xs" wrap="nowrap" style={{ minWidth: 0 }}>
          <div style={{ minWidth: 0 }}>
            <Title order={4}>{entity.full_name}</Title>
            {entity.company_name && <Text c="dimmed">{entity.company_name}</Text>}
          </div>
          {!entity.is_active && <Badge color="gray">Inactive</Badge>}
          {entity.job_status && entity.job_status !== 'None' && <Badge variant="light">{entity.job_status}</Badge>}
        </Group>
        <Group gap="xs" wrap="nowrap">
          <Button size="xs" variant="default" leftSection={<IconPencil size={14} />} onClick={onEdit}>
            Edit
          </Button>
          {onAddJob && !entity.parent_id && (
            <Button size="xs" variant="default" onClick={onAddJob}>
              Add job
            </Button>
          )}
          <Menu position="bottom-end">
            <Menu.Target>
              <Button size="xs" rightSection={<IconChevronDown size={14} />}>
                New transaction
              </Button>
            </Menu.Target>
            <Menu.Dropdown>
              {cfg.newTxns.map((t) => (
                <Menu.Item key={t.label} onClick={() => open(t.path)}>
                  {t.label}
                </Menu.Item>
              ))}
            </Menu.Dropdown>
          </Menu>
        </Group>
      </Group>
      <SimpleGrid cols={{ base: 1, sm: 3 }} spacing="sm">
        <Card withBorder padding="xs">
          <Text size="xs" c="dimmed">
            Contact
          </Text>
          <Text size="sm">{entity.contact ?? ([entity.first_name, entity.last_name].filter(Boolean).join(' ') || '—')}</Text>
          {entity.phone && <Text size="sm">{entity.phone}</Text>}
          {entity.email && (
            <Text size="sm" component="a" href={`mailto:${entity.email}`}>
              {entity.email}
            </Text>
          )}
        </Card>
        <Card withBorder padding="xs">
          <Text size="xs" c="dimmed">
            Bill to
          </Text>
          {address.length ? address.map((l) => <Text key={l} size="sm">{l}</Text>) : <Text size="sm">—</Text>}
        </Card>
        {cfg.balance && (
          <Card withBorder padding="xs" data-testid="name-balance">
            <Text size="xs" c="dimmed">
              Balance
            </Text>
            <Text size="xl" fw={700} className={classes.num}>
              {formatMoney(balance.own, { symbol: true })}
            </Text>
            {balance.total !== balance.own && (
              <Text size="sm" c="dimmed" className={classes.num}>
                {formatMoney(balance.total, { symbol: true })} with jobs
              </Text>
            )}
          </Card>
        )}
      </SimpleGrid>
      <Text fw={600}>Transactions</Text>
      {txns.isLoading ? (
        <Skeleton h={200} />
      ) : txns.error ? (
        <Alert color="red">{describeError(txns.error)}</Alert>
      ) : (txns.data ?? []).length === 0 ? (
        <Text c="dimmed" size="sm">
          No transactions yet.
        </Text>
      ) : (
        <Table.ScrollContainer minWidth={phone ? 0 : 520}>
          <Table highlightOnHover striped fz="sm" verticalSpacing={4}>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Type</Table.Th>
                <Table.Th>Num</Table.Th>
                <Table.Th>Date</Table.Th>
                {!phone && <Table.Th>Account</Table.Th>}
                <Table.Th ta="right">Amount</Table.Th>
                {cfg.openTypes.length > 0 && <Table.Th ta="right">Open</Table.Th>}
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {(txns.data ?? []).map((t) => (
                <Table.Tr key={t.id} className={classes.clickable} onClick={() => open(paths.txn(t.id))}>
                  <Table.Td>{TXN_LABEL[t.txn_type]}</Table.Td>
                  <Table.Td>{t.number}</Table.Td>
                  <Table.Td>{formatDateShort(t.date)}</Table.Td>
                  {!phone && (
                    <Table.Td>
                      <Text size="sm" truncate maw={220}>
                        {t.account_name}
                      </Text>
                    </Table.Td>
                  )}
                  <Table.Td ta="right" className={classes.num}>
                    {formatMoney(t.amount)}
                  </Table.Td>
                  {cfg.openTypes.length > 0 && (
                    <Table.Td ta="right" className={classes.num}>
                      {cfg.openTypes.includes(t.txn_type) && t.open_balance ? formatMoney(t.open_balance) : ''}
                    </Table.Td>
                  )}
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
        </Table.ScrollContainer>
      )}
    </Stack>
  );
}

