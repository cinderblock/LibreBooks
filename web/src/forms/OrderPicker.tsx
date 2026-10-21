// A name's open orders of a kind, to pick from: QBD's "Open Purchase Orders"
// (Select PO on a receipt or bill, several at once) and "Available Sales
// Orders" (a new invoice for a customer with open sales orders, one at a time).
// Openness is derived from the linked documents (`useOpenOrders`).

import { Alert, Button, Checkbox, Group, Loader, Modal, Stack, Table, Text } from '@mantine/core';
import { useState } from 'react';
import { describeError } from '~/api/errors';
import { useOpenOrders, type OpenOrder } from '~/api/hooks';
import { formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import type { OrderType } from '~/lib/orders';
import { TXN_LABEL } from '~/lib/txnTypes';
import classes from './forms.module.css';

export interface OrderPickerProps {
  opened: boolean;
  type: OrderType;
  entityId: string | null;
  /** Several at once (Select PO) or one (an invoice draws on one order at a time). */
  multiple: boolean;
  onClose(): void;
  onPick(orders: OpenOrder[]): void;
}

export function OrderPickerModal({ opened, type, entityId, multiple, onClose, onPick }: OrderPickerProps) {
  const title = type === 'PurchaseOrder' ? 'Open Purchase Orders' : type === 'SalesOrder' ? 'Available Sales Orders' : 'Open Estimates';
  return (
    <Modal opened={opened} onClose={onClose} title={title} size="lg" centered>
      {opened && <Body type={type} entityId={entityId} multiple={multiple} onClose={onClose} onPick={onPick} />}
    </Modal>
  );
}

function Body({ type, entityId, multiple, onClose, onPick }: Omit<OrderPickerProps, 'opened'>) {
  const orders = useOpenOrders(type, entityId);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const rows = orders.data ?? [];
  const toggle = (id: string) =>
    setPicked((s) => {
      const next = multiple ? new Set(s) : new Set<string>();
      if (s.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const confirm = () => onPick(rows.filter((o) => picked.has(o.order.id)));
  if (orders.isLoading) return <Loader size="sm" />;
  if (orders.error) return <Alert color="red">{describeError(orders.error)}</Alert>;
  if (rows.length === 0) {
    return (
      <Stack>
        <Text c="dimmed">No open {TXN_LABEL[type].toLowerCase()}s for this name.</Text>
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Close
          </Button>
        </Group>
      </Stack>
    );
  }
  return (
    <Stack gap="sm" data-testid="order-picker">
      <Table fz="sm" withTableBorder highlightOnHover aria-label={`Open ${TXN_LABEL[type].toLowerCase()}s`}>
        <Table.Thead>
          <Table.Tr>
            <Table.Th w={36} />
            <Table.Th>Date</Table.Th>
            <Table.Th>No.</Table.Th>
            <Table.Th>Memo</Table.Th>
            <Table.Th ta="right">Amount</Table.Th>
            <Table.Th ta="right">Remaining</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {rows.map(({ order, progress }) => (
            <Table.Tr
              key={order.id}
              className={classes.selectable}
              data-selected={picked.has(order.id) || undefined}
              onClick={() => toggle(order.id)}
              onDoubleClick={() => onPick([{ order, progress }])}
              data-testid="order-row"
            >
              <Table.Td>
                <Checkbox size="xs" checked={picked.has(order.id)} onChange={() => toggle(order.id)} aria-label={`${TXN_LABEL[type]} ${order.number ?? ''}`.trim()} onClick={(e) => e.stopPropagation()} />
              </Table.Td>
              <Table.Td>{formatDateShort(order.date)}</Table.Td>
              <Table.Td>{order.number}</Table.Td>
              <Table.Td>{order.memo}</Table.Td>
              <Table.Td ta="right" className={classes.total}>
                {formatMoney(progress.amount)}
              </Table.Td>
              <Table.Td ta="right" className={classes.total}>
                {formatMoney(progress.amountRemaining)}
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
      <Group justify="flex-end" gap="xs">
        <Button variant="default" onClick={onClose}>
          Cancel
        </Button>
        <Button onClick={confirm} disabled={picked.size === 0}>
          {multiple ? 'Add selected' : 'Continue'}
        </Button>
      </Group>
    </Stack>
  );
}
