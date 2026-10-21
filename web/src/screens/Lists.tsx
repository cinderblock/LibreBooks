// The smaller list windows (Lists menu): items, classes, terms, payment methods,
// sales tax codes, other names. Each is QBD's list: Enter or Ctrl+E edits the
// selected record in its editor window, Ctrl+N starts a new one, Ctrl+D deletes
// (the server refuses what is in use), Ctrl+Q runs the QuickReport where there
// is one.

import { ActionIcon, Alert, Button, Checkbox, Group, Skeleton, Text, TextInput } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconPencil, IconPlus, IconSearch, IconTrash } from '@tabler/icons-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { describeError } from '~/api/errors';
import { useAccountIndex, useDeleteRecord, useList } from '~/api/hooks';
import type { ListName, Terms } from '~/api/types';
import { KeyList, type KeyListColumn } from '~/components/KeyList';
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
  'sales-tax-codes': 'Sales Tax Code List',
  'other-names': 'Other Names List',
};

interface Row {
  id: string;
  name: string;
  depth: number;
  active: boolean;
  edit_seq: number;
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
    case 'sales-tax-codes':
      return <SimpleList list="sales-tax-codes" />;
    case 'other-names':
      return <OtherNames />;
    default:
      return <Alert m="md">There's no list called "{list}".</Alert>;
  }
}

interface FrameProps {
  /** The API list the rows belong to (editors, deletes). */
  list: ListName;
  rows: Row[];
  columns: KeyListColumn<Row>[];
  loading: boolean;
  error: unknown;
  /** Presets for a new record's editor (`kind` for names). */
  newParams?: { kind?: string };
  /** What the record is called in confirmations. */
  noun: string;
  /** Ctrl+Q, when the list has a QuickReport. */
  quickReport?(r: Row): string;
  testId: string;
}

function Frame({ list, rows, columns, loading, error, newParams, noun, quickReport, testId }: FrameProps) {
  const win = useWindow();
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const del = useDeleteRecord(list);
  const [query, setQuery] = useState('');
  const [inactive, setInactive] = useState(false);
  const [selected, setSelected] = useState(0);
  const shown = rows.filter((r) => (inactive || r.active) && (!query || r.name.toLowerCase().includes(query.toLowerCase())));
  const current = shown[selected];

  const create = () => open(paths.newRecord(list, newParams));
  const edit = (r: Row) => open(paths.record(list, r.id));
  const remove = async (r: Row) => {
    if (!window.confirm(`Delete the ${noun} "${r.name}"?`)) return;
    try {
      await del.mutateAsync({ id: r.id, editSeq: r.edit_seq });
      notifications.show({ message: `Deleted ${r.name}.` });
    } catch (e) {
      // 409 `in_use`: the server says what still refers to it (QBD: make it inactive instead).
      notifications.show({ color: 'red', title: "Can't delete", message: describeError(e) });
    }
  };

  const enabled = win.isActiveNow;
  useShortcut('record.new', create, { enabled });
  useShortcut('record.edit', () => (current ? edit(current) : false), { enabled });
  useShortcut('record.delete', () => (current ? void remove(current) : false), { enabled });
  useShortcut('report.quick', () => (current && quickReport ? open(quickReport(current)) : false), { enabled: () => enabled() && !!quickReport });

  if (error) return <Alert m="md" color="red">{describeError(error)}</Alert>;
  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid={testId}>
      <Group gap="xs" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0} wrap="nowrap">
        <TextInput
          placeholder="Find"
          leftSection={<IconSearch size={14} />}
          value={query}
          onChange={(e) => {
            setQuery(e.currentTarget.value);
            setSelected(0);
          }}
          style={{ flex: mobile ? 1 : '0 1 260px' }}
          aria-label="Find"
        />
        {!mobile && <Checkbox label="Include inactive" checked={inactive} onChange={(e) => setInactive(e.currentTarget.checked)} />}
        {mobile ? (
          <ActionIcon variant="light" size="lg" aria-label="New" onClick={create}>
            <IconPlus size={18} />
          </ActionIcon>
        ) : (
          <Group gap="xs" ml="auto" wrap="nowrap">
            <Button variant="default" leftSection={<IconPencil size={16} />} onClick={() => current && edit(current)} disabled={!current}>
              Edit
            </Button>
            <Button variant="default" color="red" leftSection={<IconTrash size={16} />} onClick={() => current && void remove(current)} disabled={!current}>
              Delete
            </Button>
            <Button variant="light" leftSection={<IconPlus size={16} />} onClick={create}>
              New
            </Button>
          </Group>
        )}
      </Group>
      {mobile && <Checkbox px="sm" size="xs" label="Include inactive" checked={inactive} onChange={(e) => setInactive(e.currentTarget.checked)} />}
      {loading ? (
        <Skeleton h={300} />
      ) : (
        <KeyList
          label={LIST_TITLES[list] ?? 'List'}
          items={shown}
          rowKey={(r) => r.id}
          selected={selected}
          onSelect={setSelected}
          onOpen={edit}
          jumpText={(r) => r.name}
          mobile={mobile}
          autoFocus={!mobile}
          empty={<Text c="dimmed" p="md">Nothing here yet. {mobile ? 'Tap +' : 'Press Ctrl+N or New'} to add one.</Text>}
          rowAttrs={(r) => ({ 'data-inactive': !r.active || undefined })}
          columns={[
            {
              key: 'name',
              header: 'Name',
              width: 'minmax(200px, 2fr)',
              render: (r) => (
                <span style={{ paddingLeft: query ? 0 : r.depth * 16 }}>
                  {r.name}
                  {!r.active ? ' (inactive)' : ''}
                </span>
              ),
            },
            ...columns,
          ]}
          renderMobile={(r) => (
            <>
              <span style={{ paddingLeft: r.depth * 14, minWidth: 0 }}>
                <Text size="sm" truncate>
                  {r.name}
                </Text>
                <Text size="xs" c="dimmed" truncate>
                  {r.cols[0]}
                </Text>
              </span>
            </>
          )}
        />
      )}
    </div>
  );
}

const col = (i: number, header: string, width: string, align?: 'right'): KeyListColumn<Row> => ({ key: header, header, width, align, render: (r) => r.cols[i] });

function ItemsList() {
  const { data = [], isLoading, error } = useList('items', { include_inactive: true });
  const accounts = useAccountIndex();
  const rows = useMemo<Row[]>(
    () =>
      data.map((i) => ({
        id: i.id,
        name: i.name,
        depth: i.sublevel,
        active: i.is_active,
        edit_seq: i.edit_seq,
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
  return (
    <Frame
      list="items"
      noun="item"
      testId="item-list"
      rows={rows}
      loading={isLoading}
      error={error}
      columns={[col(0, 'Type', '140px'), col(1, 'Description', 'minmax(200px, 2fr)'), col(2, 'Account', 'minmax(160px, 1.5fr)'), col(3, 'Price', '90px', 'right'), col(4, 'Cost', '90px', 'right')]}
    />
  );
}

/** QBD's Terms List "Detail" column. */
export function describeTerms(t: Terms): string {
  const k = t.kind;
  if (k.type === 'Standard') return `${k.due_days === 0 ? 'Due on receipt' : `Net ${k.due_days} days`}${k.discount_percent ? ` · ${k.discount_percent}% if paid in ${k.discount_days} days` : ''}`;
  return `Due on the ${ordinal(k.day_of_month)}${k.discount_percent ? ` · ${k.discount_percent}% if paid by the ${ordinal(k.discount_day_of_month ?? k.day_of_month)}` : ''}`;
}

function ordinal(n: number): string {
  const s = ['th', 'st', 'nd', 'rd'];
  const v = n % 100;
  return `${n}${s[(v - 20) % 10] ?? s[v] ?? s[0]}`;
}

const SIMPLE_NOUN: Record<'classes' | 'terms' | 'payment-methods' | 'sales-tax-codes', string> = { classes: 'class', terms: 'terms', 'payment-methods': 'payment method', 'sales-tax-codes': 'sales tax code' };

function SimpleList({ list }: { list: 'classes' | 'terms' | 'payment-methods' | 'sales-tax-codes' }) {
  const { data = [], isLoading, error } = useList(list, { include_inactive: true });
  const rows = useMemo<Row[]>(
    () =>
      data.map((r) => {
        const rec = r as { id: string; name: string; is_active: boolean; edit_seq: number; sublevel?: number; kind?: unknown; description?: string | null; is_taxable?: boolean };
        let detail = '';
        if (list === 'terms') detail = describeTerms(r as Terms);
        else if (list === 'payment-methods') detail = String(rec.kind ?? '');
        else if (list === 'sales-tax-codes') detail = `${rec.is_taxable ? 'Taxable' : 'Non-taxable'}${rec.description ? ` · ${rec.description}` : ''}`;
        return { id: rec.id, name: rec.name, depth: rec.sublevel ?? 0, active: rec.is_active, edit_seq: rec.edit_seq, cols: [detail] };
      }),
    [data, list],
  );
  return <Frame list={list} noun={SIMPLE_NOUN[list]} testId={`${list}-list`} rows={rows} loading={isLoading} error={error} columns={list === 'classes' ? [] : [{ key: 'detail', header: list === 'payment-methods' ? 'Type' : 'Detail', width: 'minmax(160px, 2fr)', render: (r) => r.cols[0] }]} />;
}

function OtherNames() {
  const { data = [], isLoading, error } = useList('entities', { include_inactive: true, kind: 'OtherName' });
  const rows = useMemo<Row[]>(() => data.map((e) => ({ id: e.id, name: e.full_name, depth: 0, active: e.is_active, edit_seq: e.edit_seq, cols: [e.phone ?? '', e.email ?? ''] })), [data]);
  return (
    <Frame
      list="entities"
      newParams={{ kind: 'OtherName' }}
      noun="name"
      testId="other-names-list"
      rows={rows}
      loading={isLoading}
      error={error}
      quickReport={(r) => paths.report('TransactionListByDate', { name: r.id, dates: 'All' })}
      columns={[col(0, 'Phone', '160px'), col(1, 'Email', 'minmax(160px, 1fr)')]}
    />
  );
}
