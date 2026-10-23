// What has been drawn on a saved order, under its form: per line what was
// ordered, what the linked invoices (or receipts and bills) took and what is
// left, which lines are closed; the documents themselves with a way into each;
// Open / Closed; Mark as closed / Reopen (QBD's "Closed" stamp,
// `Txn.is_manually_closed`); and the buttons that make the next invoice, sales
// order, receipt or bill from it. The figures are the server's
// (`GET /txns/:id/fulfilment`), presented by `lib/orders.ts`.

import { Badge, Button, Group, Loader, Table, Text, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { useItemIndex, useOrderProgress, useSaveTxn, useTxn } from '~/api/hooks';
import { describeError } from '~/api/errors';
import type { Txn, TxnType } from '~/api/types';
import { formatDecimal } from '~/lib/decimal';
import { formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { isOrderType, linkedOrderIds, progressMeasure, type OrderProgress } from '~/lib/orders';
import { TXN_LABEL } from '~/lib/txnTypes';
import { paths } from '~/windows/paths';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { targetTypes } from './linked';
import classes from './forms.module.css';

/** The verb for what draws on each kind of order. */
export function drawnLabel(orderType: TxnType): string {
  return orderType === 'PurchaseOrder' ? 'Received' : 'Invoiced';
}

const CREATE_LABEL: Partial<Record<TxnType, string>> = { Invoice: 'Create Invoice', SalesOrder: 'Create Sales Order', ItemReceipt: 'Create Item Receipt', Bill: 'Enter Bill' };

export function createLabel(target: TxnType): string {
  return CREATE_LABEL[target] ?? `Create ${TXN_LABEL[target]}`;
}

/** The order's status as QBD words it. */
export function orderStatus(p: OrderProgress, orderType: TxnType): { label: string; color: string } {
  if (p.manuallyClosed) return { label: 'Closed', color: 'gray' };
  if (p.fullyFulfilled) return { label: orderType === 'PurchaseOrder' ? 'Closed: fully received' : 'Closed: fully invoiced', color: 'gray' };
  if (p.amountFulfilled > 0) return { label: orderType === 'PurchaseOrder' ? 'Partly received' : 'Partly invoiced', color: 'blue' };
  return { label: 'Open', color: 'teal' };
}

/** "From Sales Order 1001" on a document whose lines draw on an order (or a receipt), with a way into it. */
export function DrawnFromNote({ txn }: { txn: Txn | null }) {
  const ids = txn ? linkedOrderIds(txn) : [];
  const open = useOpenWindow();
  // One order at a time is the rule on our forms; an import may carry more, and the first is named.
  const first = useTxn(ids[0] ?? null);
  if (!ids.length) return null;
  const order = first.data?.txn;
  const label = order ? `${TXN_LABEL[order.txn_type]}${order.number ? ` ${order.number}` : ''}` : 'an order';
  return (
    <Badge variant="light" color="blue" data-testid="drawn-from" style={{ cursor: 'pointer', textTransform: 'none' }} onClick={() => open(paths.txn(ids[0]!))} title="Open the transaction this draws on">
      From {label}
      {ids.length > 1 ? ` and ${ids.length - 1} more` : ''}
    </Badge>
  );
}

export function OrderProgressPanel({ order, dirty, onDraw }: { order: Txn; dirty: boolean; onDraw(target: TxnType, progress: OrderProgress): void }) {
  const progress = useOrderProgress(order);
  const items = useItemIndex();
  const open = useOpenWindow();
  const save = useSaveTxn();
  if (!isOrderType(order.txn_type)) return null;
  const type = order.txn_type;
  const drawn = drawnLabel(type);
  const measure = progressMeasure(type);
  if (!progress) {
    return (
      <Group gap="xs" data-testid="order-progress">
        <Loader size="xs" />
        <Text size="sm" c="dimmed">
          Looking up what has been {drawn.toLowerCase()}…
        </Text>
      </Group>
    );
  }
  const status = orderStatus(progress, type);
  const canClose = type !== 'Estimate';
  const setClosed = async (closed: boolean) => {
    try {
      await save.mutateAsync({ ...order, is_manually_closed: closed });
      notifications.show({ message: closed ? `${TXN_LABEL[type]} marked as closed.` : `${TXN_LABEL[type]} reopened.`, color: 'teal' });
    } catch (e) {
      notifications.show({ message: describeError(e), color: 'red' });
    }
  };
  const blocked = dirty ? 'Save your changes first' : progress.closed ? `This ${TXN_LABEL[type].toLowerCase()} is closed` : order.is_void ? `This ${TXN_LABEL[type].toLowerCase()} is void` : null;
  const verbing = drawn === 'Received' ? 'receiving' : 'invoicing';
  return (
    <div className={classes.progress} data-testid="order-progress">
      <Group justify="space-between" wrap="wrap" gap="xs">
        <Group gap="xs">
          <Text fw={600}>{drawn}</Text>
          <Badge color={status.color} variant="light" data-testid="order-status">
            {status.label}
          </Badge>
          <Text size="sm" c="dimmed" data-testid="order-percent">
            {formatMoney(progress.amountFulfilled)} of {formatMoney(progress.amount)} ({formatDecimal(progress.percent, 2)}%)
          </Text>
        </Group>
        <Group gap="xs">
          {canClose &&
            !order.is_void &&
            (progress.manuallyClosed ? (
              <Button size="xs" variant="default" onClick={() => void setClosed(false)} loading={save.isPending} disabled={dirty}>
                Reopen
              </Button>
            ) : (
              !progress.fullyFulfilled && (
                <Tooltip label={dirty ? 'Save your changes first' : `Close the ${TXN_LABEL[type].toLowerCase()} without ${verbing} the rest`} openDelay={600}>
                  <Button size="xs" variant="default" onClick={() => void setClosed(true)} loading={save.isPending} disabled={dirty}>
                    Mark as closed
                  </Button>
                </Tooltip>
              )
            ))}
          {targetTypes(type).map((target) => (
            <Tooltip key={target} label={blocked ?? `A new ${TXN_LABEL[target].toLowerCase()} for what remains, or part of it`} openDelay={600}>
              <Button size="xs" onClick={() => onDraw(target, progress)} disabled={!!blocked}>
                {createLabel(target)}
              </Button>
            </Tooltip>
          ))}
        </Group>
      </Group>
      <Table fz="sm" withTableBorder aria-label={`${drawn} by line`}>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Item</Table.Th>
            <Table.Th>Description</Table.Th>
            {measure === 'quantity' ? (
              <>
                <Table.Th ta="right">Ordered</Table.Th>
                <Table.Th ta="right">{drawn}</Table.Th>
                <Table.Th ta="right">Remaining</Table.Th>
              </>
            ) : (
              <>
                <Table.Th ta="right">Est. Amount</Table.Th>
                <Table.Th ta="right">Invoiced</Table.Th>
                <Table.Th ta="right">Remaining</Table.Th>
                <Table.Th ta="right">%</Table.Th>
              </>
            )}
            <Table.Th ta="center" title="Closed: nothing more is expected on this line">
              Clsd
            </Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {progress.lines.map((l) => (
            <Table.Tr key={l.line.id} data-closed={l.closed || undefined}>
              <Table.Td>{l.line.item_id ? (items.get(l.line.item_id)?.full_name ?? l.line.item_id) : ''}</Table.Td>
              <Table.Td>{l.line.description}</Table.Td>
              {measure === 'quantity' ? (
                <>
                  <Table.Td ta="right" className={classes.total}>
                    {formatDecimal(l.ordered)}
                  </Table.Td>
                  <Table.Td ta="right" className={classes.total} data-testid="line-fulfilled">
                    {formatDecimal(l.fulfilled)}
                  </Table.Td>
                  <Table.Td ta="right" className={classes.total} data-testid="line-remaining">
                    {formatDecimal(l.remaining)}
                  </Table.Td>
                </>
              ) : (
                <>
                  <Table.Td ta="right" className={classes.total}>
                    {formatMoney(l.amount)}
                  </Table.Td>
                  <Table.Td ta="right" className={classes.total} data-testid="line-fulfilled">
                    {formatMoney(l.amountFulfilled)}
                  </Table.Td>
                  <Table.Td ta="right" className={classes.total} data-testid="line-remaining">
                    {formatMoney(l.amountRemaining)}
                  </Table.Td>
                  <Table.Td ta="right" className={classes.total}>
                    {l.amount ? `${formatDecimal({ int: (BigInt(l.amountFulfilled) * 10_000n) / BigInt(l.amount), scale: 2 }, 2)}%` : ''}
                  </Table.Td>
                </>
              )}
              <Table.Td ta="center" data-testid="line-closed" aria-label={l.closed ? 'Closed' : 'Open'}>
                {l.closed ? '✓' : ''}
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
      {progress.linked.length > 0 && (
        <Group gap="xs" wrap="wrap">
          <Text size="sm" c="dimmed">
            {drawn} on:
          </Text>
          {progress.linked.map((t) => (
            <Button key={t.id} size="compact-xs" variant="light" onClick={() => open(paths.txn(t.id))} aria-label={`Open ${TXN_LABEL[t.txn_type]} ${t.number ?? ''}`.trim()}>
              {TXN_LABEL[t.txn_type]} {t.number ?? ''} · {formatDateShort(t.date)} · {formatMoney(t.amount)}
            </Button>
          ))}
        </Group>
      )}
    </div>
  );
}
