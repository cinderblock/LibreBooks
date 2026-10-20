// Memorized Transaction List (Lists → Memorized Transaction List, Ctrl+T): what
// QBD shows there — every memorized transaction with its schedule, the ones
// due now on top, Enter Now (the template becomes a fresh, unsaved transaction
// open in its form), Edit Schedule (the Memorize dialog again) and Delete.

import { Alert, Badge, Button, Card, Group, Stack, Table, Text, Title } from '@mantine/core';
import { IconClockPlay, IconPencil, IconTrash } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useDeleteSaved, useEntityIndex, useMemorizedDue, useSaved } from '~/api/hooks';
import type { Memorized, Saved } from '~/api/types';
import { INTERVAL_LABEL, MemorizeModal, REPEAT_LABEL } from '~/forms/MemorizeModal';
import { formSlugFor } from '~/forms/registry';
import { formatDate } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { stashTxn } from '~/windows/handoff';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { Confirm } from './admin/common';
import classes from './screens.module.css';

type Row = { saved: Saved; m: Memorized; due: boolean };

export function MemorizedList() {
  const win = useWindow();
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const open = useOpenWindow();
  const list = useSaved('memorized_txn');
  const due = useMemorizedDue();
  const entities = useEntityIndex();
  const del = useDeleteSaved('memorized_txn');
  const [editing, setEditing] = useState<Row | null>(null);
  const [deleting, setDeleting] = useState<Row | null>(null);
  useEffect(() => win.setTitle('Memorized Transaction List'), []); // eslint-disable-line react-hooks/exhaustive-deps

  const dueIds = useMemo(() => new Set((due.data ?? []).map((d) => d.id)), [due.data]);
  const rows: Row[] = useMemo(() => {
    const all = (list.data ?? []).map((s) => ({ saved: s, m: s.doc as Memorized, due: dueIds.has(s.id) }));
    // Due first, then by group and name (QBD groups are headings; here a column).
    return all.sort((a, b) => Number(b.due) - Number(a.due) || (a.m.group ?? '').localeCompare(b.m.group ?? '') || a.saved.name.localeCompare(b.saved.name));
  }, [list.data, dueIds]);

  const enter = useMutation({
    mutationFn: (row: Row) => api.useMemorized(co, row.saved.id),
    onSuccess: (txn) => {
      const token = stashTxn(txn);
      const slug = formSlugFor(txn.txn_type);
      open(txn.txn_type === 'Check' || !slug ? paths.newCheck(token) : paths.form(slug, token));
    },
  });

  const payee = (m: Memorized) => (m.template.entity_id ? (entities.get(m.template.entity_id)?.full_name ?? '') : '');
  const schedule = (m: Memorized) => (m.repeat === 'Never' ? REPEAT_LABEL.Never : `${REPEAT_LABEL[m.repeat]} · ${m.interval ? INTERVAL_LABEL[m.interval] : ''}`);

  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="memorized-list">
      <Group justify="space-between" align="flex-start">
        <div>
          <Title order={3}>Memorized transactions</Title>
          <Text size="sm" c="dimmed">
            Memorize any form with Ctrl+M. Ones set to remind you appear here when due; "Enter now" opens the form filled in, dated today.
          </Text>
        </div>
        {dueIds.size > 0 && <Badge color="orange" size="lg">{dueIds.size} due</Badge>}
      </Group>
      {enter.error && <Alert color="red">{describeError(enter.error)}</Alert>}
      {rows.length === 0 && !list.isLoading && (
        <Card withBorder>
          <Text c="dimmed" size="sm">
            Nothing memorized yet. Open a form you enter often (rent, a loan payment, a monthly invoice), fill it in, and press Ctrl+M.
          </Text>
        </Card>
      )}
      {rows.length > 0 && (
        <Table fz="sm" withTableBorder highlightOnHover>
          <Table.Thead>
            <Table.Tr>
              <Table.Th>Name</Table.Th>
              {!mobile && <Table.Th>Type</Table.Th>}
              {!mobile && <Table.Th>Payee / customer</Table.Th>}
              <Table.Th ta="right">Amount</Table.Th>
              <Table.Th>Next</Table.Th>
              {!mobile && <Table.Th>Schedule</Table.Th>}
              <Table.Th />
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {rows.map((r) => (
              <Table.Tr key={r.saved.id} data-testid="memorized-row" data-due={r.due || undefined}>
                <Table.Td>
                  <Group gap="xs" wrap="nowrap">
                    <Text size="sm">{r.saved.name}</Text>
                    {r.due && (
                      <Badge color="orange" size="xs">
                        due
                      </Badge>
                    )}
                  </Group>
                  {r.m.group && (
                    <Text size="xs" c="dimmed">
                      {r.m.group}
                    </Text>
                  )}
                </Table.Td>
                {!mobile && <Table.Td>{TXN_LABEL[r.m.template.txn_type]}</Table.Td>}
                {!mobile && <Table.Td>{payee(r.m)}</Table.Td>}
                <Table.Td ta="right" className={classes.num}>
                  {formatMoney(Math.abs(r.m.template.amount))}
                </Table.Td>
                <Table.Td>
                  {r.m.next_date ? formatDate(r.m.next_date) : <Text span c="dimmed">—</Text>}
                  {r.m.remaining !== null && (
                    <Text size="xs" c="dimmed">
                      {r.m.remaining} left
                    </Text>
                  )}
                </Table.Td>
                {!mobile && <Table.Td>{schedule(r.m)}</Table.Td>}
                <Table.Td ta="right">
                  <Group gap={4} justify="flex-end" wrap="nowrap">
                    <Button size="compact-xs" leftSection={<IconClockPlay size={12} />} loading={enter.isPending && enter.variables?.saved.id === r.saved.id} onClick={() => enter.mutate(r)}>
                      Enter now
                    </Button>
                    {!mobile && (
                      <Button size="compact-xs" variant="subtle" leftSection={<IconPencil size={12} />} onClick={() => setEditing(r)}>
                        Schedule
                      </Button>
                    )}
                    <Button size="compact-xs" variant="subtle" color="red" aria-label={`Delete ${r.saved.name}`} onClick={() => setDeleting(r)}>
                      <IconTrash size={12} />
                    </Button>
                  </Group>
                </Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      )}
      <MemorizeModal txn={editing?.m.template ?? null} existing={editing ? { id: editing.saved.id, name: editing.saved.name, memorized: editing.m } : null} onClose={() => setEditing(null)} />
      <Confirm
        opened={!!deleting}
        title={`Delete "${deleting?.saved.name}"?`}
        confirmLabel="Delete"
        loading={del.isPending}
        onConfirm={() => deleting && del.mutate(deleting.saved.id, { onSuccess: () => setDeleting(null) })}
        onClose={() => setDeleting(null)}
      >
        <Text size="sm">Only the memorized template goes; transactions already entered from it stay.</Text>
        {del.error && <Alert color="red" mt="sm">{describeError(del.error)}</Alert>}
      </Confirm>
    </Stack>
  );
}
