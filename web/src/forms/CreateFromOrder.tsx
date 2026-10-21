// "Create Invoice" from a sales order or estimate, "Create Item Receipt" /
// "Enter Bill" from a purchase order: QBD's dialog asking how much of the
// order to draw. Sales and purchase orders: everything that remains, or
// selected items and quantities. Estimates: the entire estimate (100%), a
// percentage of the whole, or selected items with an amount or percent per
// line. The result is a new, unsaved document (`forms/linked.ts`) the caller
// hands to its form.

import { Button, Group, Modal, Radio, Stack, Table, Text, TextInput } from '@mantine/core';
import { useMemo, useState } from 'react';
import { useItemIndex } from '~/api/hooks';
import type { Txn, TxnType } from '~/api/types';
import { MoneyField } from '~/inputs/MoneyField';
import { formatDecimal, toApiString } from '~/lib/decimal';
import { todayIso } from '~/lib/dates';
import { formatMoney, type Cents } from '~/lib/money';
import { progressMeasure, type OrderProgress, type OrderType } from '~/lib/orders';
import { TXN_LABEL } from '~/lib/txnTypes';
import { amountToPercent, documentFromOrder, drawAmounts, drawPercent, drawPercentOfTotal, drawQuantities, drawRemaining, drawTotal, percentToAmount, type Draw, type ProgressChoice } from './linked';
import { createLabel, drawnLabel } from './OrderProgress';
import { decimalTextError } from './purchaseLines';
import classes from './forms.module.css';

export interface CreateFromOrderProps {
  /** The order being drawn on (null closes the dialog). */
  order: Txn | null;
  progress: OrderProgress | null;
  /** What to make: an invoice, an item receipt or a bill. */
  target: TxnType;
  onClose(): void;
  onCreate(doc: Txn): void;
}

export function CreateFromOrderModal({ order, progress, target, onClose, onCreate }: CreateFromOrderProps) {
  return (
    <Modal opened={!!order && !!progress} onClose={onClose} title={order ? `${createLabel(target)} from ${TXN_LABEL[order.txn_type]}${order.number ? ` ${order.number}` : ''}` : ''} size="xl" centered>
      {order && progress && <Body key={order.id} order={order} progress={progress} target={target} onClose={onClose} onCreate={onCreate} />}
    </Modal>
  );
}

function Body({ order, progress, target, onClose, onCreate }: { order: Txn; progress: OrderProgress; target: TxnType; onClose(): void; onCreate(doc: Txn): void }) {
  const items = useItemIndex();
  const type = order.txn_type as OrderType;
  const measure = progressMeasure(type);
  const drawn = drawnLabel(type);
  const started = progress.amountFulfilled > 0;
  const [choice, setChoice] = useState<ProgressChoice>('all');
  const [percent, setPercent] = useState('');
  // "Selected": the quantity (or amount) to take per line, starting from what remains.
  const [quantities, setQuantities] = useState<Map<string, string>>(() => new Map(progress.lines.map((l) => [l.line.id, toApiString(l.remaining)])));
  const [amounts, setAmounts] = useState<Map<string, Cents | null>>(() => new Map(progress.lines.map((l) => [l.line.id, l.amountRemaining || null])));

  const draw = useMemo<Draw>(() => {
    if (choice === 'all') return drawRemaining(progress);
    if (choice === 'percent') return drawPercent(progress, percent);
    return measure === 'amount' ? drawAmounts(progress, amounts) : drawQuantities(progress, quantities);
  }, [choice, percent, quantities, amounts, progress, measure]);
  const total = drawTotal(draw);
  const pct = drawPercentOfTotal(progress, draw);
  const percentError = choice === 'percent' && percent.trim() !== '' ? decimalTextError(percent) : undefined;
  const nothing = draw.size === 0;
  const verb = target === 'Invoice' ? 'invoice' : 'receive';

  const create = () => {
    if (nothing) return;
    onCreate(documentFromOrder(order, progress, draw, target, todayIso()));
  };

  const itemName = (id: string | null) => (id ? (items.get(id)?.full_name ?? id) : '');

  return (
    <Stack gap="sm" data-testid="create-from-order">
      <Radio.Group value={choice} onChange={(v) => setChoice(v as ProgressChoice)} aria-label={`How much to ${verb}`}>
        <Stack gap={6}>
          <Radio value="all" label={measure === 'amount' ? (started ? 'What remains of the estimate' : 'The entire estimate (100%)') : `Everything that remains on the ${TXN_LABEL[type].toLowerCase()}`} />
          {measure === 'amount' && (
            <Group gap="xs" align="center" wrap="nowrap">
              <Radio value="percent" label="A percentage of the entire estimate" />
              <TextInput
                size="xs"
                w={90}
                inputMode="decimal"
                value={percent}
                onChange={(e) => {
                  setPercent(e.currentTarget.value);
                  setChoice('percent');
                }}
                onFocus={() => setChoice('percent')}
                rightSection={<Text size="xs">%</Text>}
                error={percentError}
                aria-label="Percent of the estimate"
                classNames={{ input: 'lb-num' }}
              />
            </Group>
          )}
          <Radio value="selected" label={measure === 'amount' ? 'Selected items, or different percentages of each item' : 'Selected items and quantities'} />
        </Stack>
      </Radio.Group>

      <Table fz="sm" withTableBorder aria-label="Lines to draw">
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Item</Table.Th>
            <Table.Th>Description</Table.Th>
            {measure === 'quantity' ? (
              <>
                <Table.Th ta="right">Ordered</Table.Th>
                <Table.Th ta="right">{drawn}</Table.Th>
                <Table.Th ta="right">Remaining</Table.Th>
                <Table.Th ta="right" w={110}>
                  To {verb}
                </Table.Th>
              </>
            ) : (
              <>
                <Table.Th ta="right">Est. Amt</Table.Th>
                <Table.Th ta="right">Prior Amt</Table.Th>
                <Table.Th ta="right">Prior %</Table.Th>
                <Table.Th ta="right" w={130}>
                  Amount
                </Table.Th>
                <Table.Th ta="right" w={90}>
                  Curr %
                </Table.Th>
              </>
            )}
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {progress.lines.map((l, i) => {
            const d = draw.get(l.line.id);
            const n = i + 1;
            return (
              <Table.Tr key={l.line.id} data-drawn={d ? true : undefined}>
                <Table.Td>{itemName(l.line.item_id)}</Table.Td>
                <Table.Td>{l.line.description}</Table.Td>
                {measure === 'quantity' ? (
                  <>
                    <Table.Td ta="right" className={classes.total}>
                      {formatDecimal(l.ordered)}
                    </Table.Td>
                    <Table.Td ta="right" className={classes.total}>
                      {formatDecimal(l.fulfilled)}
                    </Table.Td>
                    <Table.Td ta="right" className={classes.total}>
                      {formatDecimal(l.remaining)}
                    </Table.Td>
                    <Table.Td ta="right">
                      {choice === 'selected' ? (
                        <TextInput
                          size="xs"
                          inputMode="decimal"
                          value={quantities.get(l.line.id) ?? ''}
                          onChange={(e) => {
                            const v = e.currentTarget.value;
                            setQuantities((m) => new Map(m).set(l.line.id, v));
                          }}
                          onFocus={(e) => e.currentTarget.select()}
                          error={decimalTextError(quantities.get(l.line.id) ?? '')}
                          aria-label={`To ${verb}, line ${n}`}
                          classNames={{ input: 'lb-num' }}
                        />
                      ) : (
                        <span className={classes.total}>{d ? formatDecimal(d.quantity) : ''}</span>
                      )}
                    </Table.Td>
                  </>
                ) : (
                  <>
                    <Table.Td ta="right" className={classes.total}>
                      {formatMoney(l.amount)}
                    </Table.Td>
                    <Table.Td ta="right" className={classes.total}>
                      {formatMoney(l.amountFulfilled)}
                    </Table.Td>
                    <Table.Td ta="right" className={classes.total}>
                      {amountToPercent(l, l.amountFulfilled) ?? ''}
                      {l.amount ? '%' : ''}
                    </Table.Td>
                    <Table.Td ta="right">
                      {choice === 'selected' ? (
                        <MoneyField size="xs" blankZero value={amounts.get(l.line.id) ?? null} onChange={(v) => setAmounts((m) => new Map(m).set(l.line.id, v))} aria-label={`Amount, line ${n}`} />
                      ) : (
                        <span className={classes.total}>{d ? formatMoney(d.amount) : ''}</span>
                      )}
                    </Table.Td>
                    <Table.Td ta="right">
                      {choice === 'selected' ? (
                        <TextInput
                          size="xs"
                          inputMode="decimal"
                          value={amountToPercent(l, amounts.get(l.line.id) ?? 0) ?? ''}
                          onChange={(e) => {
                            const amount = percentToAmount(l, e.currentTarget.value);
                            setAmounts((m) => new Map(m).set(l.line.id, amount));
                          }}
                          onFocus={(e) => e.currentTarget.select()}
                          aria-label={`Percent, line ${n}`}
                          classNames={{ input: 'lb-num' }}
                        />
                      ) : (
                        <span className={classes.total}>{d ? `${amountToPercent(l, d.amount) ?? ''}%` : ''}</span>
                      )}
                    </Table.Td>
                  </>
                )}
              </Table.Tr>
            );
          })}
        </Table.Tbody>
      </Table>

      <Group justify="space-between" align="center">
        <Text size="sm" data-testid="draw-total">
          Total to {verb}:{' '}
          <Text span fw={700} className={classes.total}>
            {formatMoney(total)}
          </Text>
          {measure === 'amount' && (
            <Text span c="dimmed">
              {' '}
              ({formatDecimal(pct, 2)}% of the estimate)
            </Text>
          )}
        </Text>
        <Group gap="xs">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={create} disabled={nothing || !!percentError} data-autofocus>
            {createLabel(target)}
          </Button>
        </Group>
      </Group>
    </Stack>
  );
}
