// A name's open orders of a kind, to pick from: QBD's "Open Purchase Orders"
// (Select PO on a receipt or bill, several at once) and "Available Sales
// Orders" (a new invoice for a customer with open sales orders, one at a time);
// and, on Enter Bills, the vendor's item receipts not yet billed ("Enter Bill
// for Received Items"). The lists are the server's (`GET /open?kind=`, where
// `open_balance` is what remains on each order; `GET /txns?linked_to=` for the
// receipts no bill links to). A pick hands the summaries back; the form loads
// each whole, with its fulfilment, before drawing on it.

import { Alert, Button, Checkbox, Group, Loader, Modal, Stack, Table, Text } from '@mantine/core';
import { useState } from 'react';
import { describeError } from '~/api/errors';
import { useOpenOrders, useUnbilledReceipts } from '~/api/hooks';
import type { TxnSummary, TxnType } from '~/api/types';
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
  onPick(orders: TxnSummary[]): void;
}

export function OrderPickerModal({ opened, type, entityId, multiple, onClose, onPick }: OrderPickerProps) {
  const title = type === 'PurchaseOrder' ? 'Open Purchase Orders' : type === 'SalesOrder' ? 'Available Sales Orders' : 'Open Estimates';
  return (
    <Modal opened={opened} onClose={onClose} title={title} size="lg" centered>
      {opened && <OrdersBody type={type} entityId={entityId} multiple={multiple} onClose={onClose} onPick={onPick} />}
    </Modal>
  );
}

function OrdersBody({ type, entityId, multiple, onClose, onPick }: Omit<OrderPickerProps, 'opened'>) {
  const orders = useOpenOrders(type, entityId);
  return <PickerBody type={type} rows={orders.data} loading={orders.isLoading} error={orders.error} multiple={multiple} remaining onClose={onClose} onPick={onPick} emptyText={`No open ${TXN_LABEL[type].toLowerCase()}s for this name.`} rowTestId="order-row" />;
}

export interface ReceiptPickerProps {
  opened: boolean;
  entityId: string | null;
  onClose(): void;
  onPick(receipts: TxnSummary[]): void;
}

/** The vendor's item receipts no bill has been entered for (QBD's Enter Bill for Received Items). */
export function ReceiptPickerModal({ opened, entityId, onClose, onPick }: ReceiptPickerProps) {
  return (
    <Modal opened={opened} onClose={onClose} title="Item Receipts to Bill" size="lg" centered>
      {opened && <ReceiptsBody entityId={entityId} onClose={onClose} onPick={onPick} />}
    </Modal>
  );
}

function ReceiptsBody({ entityId, onClose, onPick }: Omit<ReceiptPickerProps, 'opened'>) {
  const receipts = useUnbilledReceipts(entityId);
  return <PickerBody type="ItemReceipt" rows={receipts.data} loading={receipts.isLoading} error={receipts.error} multiple remaining={false} onClose={onClose} onPick={onPick} emptyText="Every item receipt for this vendor has been billed." rowTestId="receipt-row" />;
}

function PickerBody({
  type,
  rows = [],
  loading,
  error,
  multiple,
  remaining,
  emptyText,
  rowTestId,
  onClose,
  onPick,
}: {
  type: TxnType;
  rows: TxnSummary[] | undefined;
  loading: boolean;
  error: unknown;
  multiple: boolean;
  /** Show what remains on each (an order's `open_balance`). */
  remaining: boolean;
  emptyText: string;
  rowTestId: string;
  onClose(): void;
  onPick(rows: TxnSummary[]): void;
}) {
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const toggle = (id: string) =>
    setPicked((s) => {
      const next = multiple ? new Set(s) : new Set<string>();
      if (s.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const confirm = () => onPick(rows.filter((o) => picked.has(o.id)));
  if (loading) return <Loader size="sm" />;
  if (error) return <Alert color="red">{describeError(error)}</Alert>;
  if (rows.length === 0) {
    return (
      <Stack>
        <Text c="dimmed">{emptyText}</Text>
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
      <Table fz="sm" withTableBorder highlightOnHover aria-label={`${TXN_LABEL[type]}s to pick from`}>
        <Table.Thead>
          <Table.Tr>
            <Table.Th w={36} />
            <Table.Th>Date</Table.Th>
            <Table.Th>No.</Table.Th>
            <Table.Th>Memo</Table.Th>
            <Table.Th ta="right">Amount</Table.Th>
            {remaining && <Table.Th ta="right">Remaining</Table.Th>}
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {rows.map((o) => (
            <Table.Tr key={o.id} className={classes.selectable} data-selected={picked.has(o.id) || undefined} onClick={() => toggle(o.id)} onDoubleClick={() => onPick([o])} data-testid={rowTestId}>
              <Table.Td>
                <Checkbox size="xs" checked={picked.has(o.id)} onChange={() => toggle(o.id)} aria-label={`${TXN_LABEL[type]} ${o.number ?? ''}`.trim()} onClick={(e) => e.stopPropagation()} />
              </Table.Td>
              <Table.Td>{formatDateShort(o.date)}</Table.Td>
              <Table.Td>{o.number}</Table.Td>
              <Table.Td>{o.memo}</Table.Td>
              <Table.Td ta="right" className={classes.total}>
                {formatMoney(o.amount)}
              </Table.Td>
              {remaining && (
                <Table.Td ta="right" className={classes.total}>
                  {formatMoney(o.open_balance ?? 0)}
                </Table.Td>
              )}
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
