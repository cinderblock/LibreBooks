// Customer, Vendor and Employee Centers: the names on the left (customers with
// their jobs underneath, open balances alongside), the selected name's details
// and transactions on the right. On a phone: the list, then the detail as its
// own screen with a back arrow.

import { ActionIcon, Alert, Badge, Button, Card, Checkbox, Group, Menu, Modal, SimpleGrid, Skeleton, Stack, Table, Text, TextInput, Title } from '@mantine/core';
import { IconChevronDown, IconPencil, IconPlus, IconSearch } from '@tabler/icons-react';
import { useEffect, useMemo, useState } from 'react';
import { describeError } from '~/api/errors';
import { useEntities, useSaveRecord, useTxns } from '~/api/hooks';
import type { Entity, EntityKind, TxnType } from '~/api/types';
import { KeyList } from '~/components/KeyList';
import { useQuickCreate } from '~/inputs/QuickCreate';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { prepare, search } from '~/lib/quickfill';
import { rollUp } from '~/lib/tree';
import { TXN_LABEL } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

interface CenterConfig {
  kind: EntityKind;
  title: string;
  /** Transactions whose open balance makes up a name's balance (credits subtract). */
  openTypes: TxnType[];
  creditTypes: TxnType[];
  newTxns: Array<{ label: string; path: string }>;
}

const CONFIGS: Record<'customers' | 'vendors' | 'employees', CenterConfig> = {
  customers: {
    kind: 'Customer',
    title: 'Customer Center',
    openTypes: ['Invoice', 'StatementCharge', 'CreditMemo'],
    creditTypes: ['CreditMemo'],
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
    creditTypes: ['VendorCredit'],
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
    creditTypes: [],
    newTxns: [{ label: 'Write Check', path: paths.newCheck() }],
  },
};

export function Center({ which }: { which: 'customers' | 'vendors' | 'employees' }) {
  const cfg = CONFIGS[which];
  const win = useWindow();
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const qc = useQuickCreate();
  const { data: all = [], isLoading } = useEntities();
  const [query, setQuery] = useState('');
  const [showInactive, setShowInactive] = useState(false);
  const [selected, setSelected] = useState(0);
  const [mobileDetail, setMobileDetail] = useState(false);
  const [editing, setEditing] = useState<Entity | null>(null);

  const names = useMemo(() => all.filter((e) => e.kind === cfg.kind), [all, cfg.kind]);
  const prepared = useMemo(() => prepare(names.map((e) => ({ fullName: e.full_name, keys: e.company_name ? [e.company_name] : [] }))), [names]);
  const rows = useMemo(() => {
    const order = query.trim() ? search(prepared, query).sort((a, b) => a - b) : names.map((_, i) => i);
    return order.map((i) => names[i]!).filter((e) => showInactive || e.is_active);
  }, [names, prepared, query, showInactive]);

  // Open balances per name (jobs roll up into their customer), from open transactions.
  const open_ = useTxns({ types: cfg.openTypes, open_only: true }, cfg.openTypes.length > 0);
  const balances = useMemo(() => {
    const own = new Map<string, number>();
    for (const t of open_.data ?? []) {
      if (!t.entity_id || t.open_balance == null) continue;
      const sign = cfg.creditTypes.includes(t.txn_type) ? -1 : 1;
      own.set(t.entity_id, (own.get(t.entity_id) ?? 0) + sign * t.open_balance);
    }
    return rollUp(names, own);
  }, [open_.data, names, cfg.creditTypes]);

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
  useShortcut('record.new', () => void qc.createEntity('', [cfg.kind]), { enabled });
  useShortcut('record.edit', () => current && setEditing(current), { enabled });
  useShortcut('report.quick', () => current && open(paths.report('transaction_list_by_date', { entity: current.id, macro: 'All' })), { enabled });

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
        <ActionIcon variant="light" size="lg" aria-label={`New ${cfg.kind}`} onClick={() => void qc.createEntity('', [cfg.kind])}>
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
            ...(cfg.openTypes.length
              ? [{ key: 'bal', header: 'Balance Total', width: '110px', align: 'right' as const, render: (e: Entity) => formatMoney(balances.get(e.id) ?? 0, { blankZero: true }) }]
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
                {formatMoney(balances.get(e.id) ?? 0, { blankZero: true })}
              </Text>
            </>
          )}
        />
      )}
    </div>
  );

  const detail = current ? (
    <NameDetail entity={current} balance={balances.get(current.id) ?? 0} cfg={cfg} onEdit={() => setEditing(current)} phone={mobile} />
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
      {editing && <EditNameModal entity={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function NameDetail({ entity, balance, cfg, onEdit, phone }: { entity: Entity; balance: number; cfg: CenterConfig; onEdit(): void; phone: boolean }) {
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
        {cfg.openTypes.length > 0 && (
          <Card withBorder padding="xs">
            <Text size="xs" c="dimmed">
              Open balance
            </Text>
            <Text size="xl" fw={700} className={classes.num}>
              {formatMoney(balance, { symbol: true })}
            </Text>
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

function EditNameModal({ entity, onClose }: { entity: Entity; onClose(): void }) {
  const [form, setForm] = useState({
    name: entity.name,
    company_name: entity.company_name ?? '',
    phone: entity.phone ?? '',
    email: entity.email ?? '',
    is_active: entity.is_active,
  });
  const save = useSaveRecord('entities');
  const field = (k: 'name' | 'company_name' | 'phone' | 'email', label: string) => (
    <TextInput label={label} value={form[k]} onChange={(e) => setForm({ ...form, [k]: e.currentTarget.value })} required={k === 'name'} data-autofocus={k === 'name' || undefined} />
  );
  return (
    <Modal opened onClose={onClose} title={`Edit ${entity.full_name}`} centered>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate(
            {
              ...entity,
              name: form.name.trim(),
              company_name: form.company_name.trim() || null,
              phone: form.phone.trim() || null,
              email: form.email.trim() || null,
              is_active: form.is_active,
            },
            { onSuccess: onClose },
          );
        }}
      >
        <Stack gap="sm">
          {field('name', 'Name')}
          {field('company_name', 'Company')}
          {field('phone', 'Phone')}
          {field('email', 'Email')}
          <Checkbox label="Active" checked={form.is_active} onChange={(e) => setForm({ ...form, is_active: e.currentTarget.checked })} />
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
