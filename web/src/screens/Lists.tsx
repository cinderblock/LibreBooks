// The smaller list windows (Lists menu): items, classes, terms, payment methods,
// other names. Read-mostly for now; items can be added through QuickCreate.

import { Alert, Button, Checkbox, Group, Skeleton, Text, TextInput } from '@mantine/core';
import { IconPlus, IconSearch } from '@tabler/icons-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { describeError } from '~/api/errors';
import { useAccountIndex, useList } from '~/api/hooks';
import { KeyList, type KeyListColumn } from '~/components/KeyList';
import { useQuickCreate } from '~/inputs/QuickCreate';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { ITEM_TYPE_LABEL } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

export const LIST_TITLES: Record<string, string> = {
  items: 'Item List',
  classes: 'Class List',
  terms: 'Terms List',
  'payment-methods': 'Payment Method List',
  'other-names': 'Other Names List',
};

interface Row {
  id: string;
  name: string;
  depth: number;
  active: boolean;
  cols: ReactNode[];
}

export function ListWindow() {
  const win = useWindow();
  const list = win.desc.params.list ?? '';
  useEffect(() => win.setTitle(LIST_TITLES[list] ?? 'List'), [list]); // eslint-disable-line react-hooks/exhaustive-deps
  switch (list) {
    case 'items':
      return <ItemsList />;
    case 'classes':
      return <SimpleList list="classes" />;
    case 'terms':
      return <SimpleList list="terms" />;
    case 'payment-methods':
      return <SimpleList list="payment-methods" />;
    case 'other-names':
      return <OtherNames />;
    default:
      return <Alert m="md">There's no list called "{list}".</Alert>;
  }
}

function Frame({ rows, columns, loading, error, onNew, onOpen }: { rows: Row[]; columns: KeyListColumn<Row>[]; loading: boolean; error: unknown; onNew?(): void; onOpen?(r: Row): void }) {
  const mobile = useIsMobile();
  const [query, setQuery] = useState('');
  const [inactive, setInactive] = useState(false);
  const [selected, setSelected] = useState(0);
  const shown = rows.filter((r) => (inactive || r.active) && (!query || r.name.toLowerCase().includes(query.toLowerCase())));
  if (error) return <Alert m="md" color="red">{describeError(error)}</Alert>;
  return (
    <div className={mobile ? classes.mobileFill : classes.fill}>
      <Group gap="xs" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0}>
        <TextInput placeholder="Find" leftSection={<IconSearch size={14} />} value={query} onChange={(e) => setQuery(e.currentTarget.value)} style={{ flex: '0 1 260px' }} />
        <Checkbox label="Include inactive" checked={inactive} onChange={(e) => setInactive(e.currentTarget.checked)} />
        {onNew && (
          <Button ml="auto" variant="light" leftSection={<IconPlus size={16} />} onClick={onNew}>
            New
          </Button>
        )}
      </Group>
      {loading ? (
        <Skeleton h={300} />
      ) : (
        <KeyList
          label="List"
          items={shown}
          rowKey={(r) => r.id}
          selected={selected}
          onSelect={setSelected}
          onOpen={(r) => onOpen?.(r)}
          jumpText={(r) => r.name}
          mobile={mobile}
          autoFocus={!mobile}
          rowAttrs={(r) => ({ 'data-inactive': !r.active || undefined })}
          columns={[{ key: 'name', header: 'Name', width: 'minmax(200px, 2fr)', render: (r) => <span style={{ paddingLeft: query ? 0 : r.depth * 16 }}>{r.name}</span> }, ...columns]}
          renderMobile={(r) => (
            <>
              <Text size="sm" style={{ paddingLeft: r.depth * 14 }} truncate>
                {r.name}
              </Text>
              <Text size="xs" c="dimmed">
                {r.cols[0]}
              </Text>
            </>
          )}
        />
      )}
    </div>
  );
}

function ItemsList() {
  const win = useWindow();
  const { data = [], isLoading, error } = useList('items', { include_inactive: true });
  const accounts = useAccountIndex();
  const qc = useQuickCreate();
  useShortcut('record.new', () => void qc.createItem(''), { enabled: win.isActiveNow });
  const rows = useMemo<Row[]>(
    () =>
      data.map((i) => ({
        id: i.id,
        name: i.name,
        depth: i.sublevel,
        active: i.is_active,
        cols: [
          ITEM_TYPE_LABEL[i.item_type],
          i.sales_description ?? i.purchase_description ?? '',
          i.income_account_id ? (accounts.get(i.income_account_id)?.full_name ?? '') : i.expense_account_id ? (accounts.get(i.expense_account_id)?.full_name ?? '') : '',
          i.sales_price ?? '',
          i.purchase_cost ?? '',
        ],
      })),
    [data, accounts],
  );
  const col = (i: number, header: string, width: string, align?: 'right'): KeyListColumn<Row> => ({ key: header, header, width, align, render: (r) => r.cols[i] });
  return (
    <Frame
      rows={rows}
      loading={isLoading}
      error={error}
      onNew={() => void qc.createItem('')}
      columns={[col(0, 'Type', '140px'), col(1, 'Description', 'minmax(200px, 2fr)'), col(2, 'Account', 'minmax(160px, 1.5fr)'), col(3, 'Price', '90px', 'right'), col(4, 'Cost', '90px', 'right')]}
    />
  );
}

function SimpleList({ list }: { list: 'classes' | 'terms' | 'payment-methods' }) {
  const { data = [], isLoading, error } = useList(list, { include_inactive: true });
  const rows = useMemo<Row[]>(
    () =>
      data.map((r) => {
        const rec = r as { id: string; name: string; is_active: boolean; sublevel?: number; kind?: unknown };
        const kind = rec.kind as { type?: string; due_days?: number } | string | undefined;
        const detail = typeof kind === 'string' ? kind : kind?.type === 'Standard' ? `Net ${kind.due_days} days` : kind?.type ?? '';
        return { id: rec.id, name: rec.name, depth: rec.sublevel ?? 0, active: rec.is_active, cols: [detail] };
      }),
    [data],
  );
  return <Frame rows={rows} loading={isLoading} error={error} columns={list === 'classes' ? [] : [{ key: 'detail', header: 'Detail', width: 'minmax(160px, 1fr)', render: (r) => r.cols[0] }]} />;
}

function OtherNames() {
  const open = useOpenWindow();
  const { data = [], isLoading, error } = useList('entities', { include_inactive: true, kind: 'OtherName' });
  const rows = useMemo<Row[]>(() => data.map((e) => ({ id: e.id, name: e.full_name, depth: 0, active: e.is_active, cols: [e.phone ?? ''] })), [data]);
  return (
    <Frame
      rows={rows}
      loading={isLoading}
      error={error}
      onOpen={(r) => open(paths.report('TransactionListByDate', { name: r.id, dates: 'All' }))}
      columns={[{ key: 'phone', header: 'Phone', width: '160px', render: (r) => r.cols[0] }]}
    />
  );
}
