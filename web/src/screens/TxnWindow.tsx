// A saved transaction's window. Checks open in Write Checks (editable); other
// types show read-only until their forms exist, with their lines and the
// postings they made.

import { Alert, Badge, Card, Group, Skeleton, Stack, Table, Text, Title } from '@mantine/core';
import { useEffect } from 'react';
import { describeError } from '~/api/errors';
import { useAccountIndex, useEntityIndex, useItemIndex, useTxn } from '~/api/hooks';
import { formFor } from '~/forms/registry';
import { formatDate } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { useWindow } from '~/windows/WindowsProvider';
import classes from './screens.module.css';

export function TxnWindow() {
  const win = useWindow();
  const id = win.desc.params.txnId!;
  const { data, error, isLoading } = useTxn(id);
  if (isLoading) return <Skeleton m="md" h={300} />;
  if (error) return <Alert m="md" color="red">{describeError(error)}</Alert>;
  if (!data) return null;
  const form = formFor(data.txn.txn_type);
  if (form) return <>{form.render(data.txn, null)}</>;
  return <TransactionView />;
}

function TransactionView() {
  const win = useWindow();
  const { data } = useTxn(win.desc.params.txnId);
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const items = useItemIndex();
  const txn = data?.txn;
  useEffect(() => {
    if (txn) win.setTitle(`${TXN_LABEL[txn.txn_type]}${txn.number ? ` ${txn.number}` : ''}`);
  }, [txn?.id, txn?.number]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!txn || !data) return null;
  const name = (id: string | null) => (id ? (accounts.get(id)?.full_name ?? entities.get(id)?.full_name ?? items.get(id)?.full_name ?? id) : '');
  return (
    <Stack p="md" gap="sm" className={classes.page}>
      <Group gap="sm">
        <Title order={3}>{TXN_LABEL[txn.txn_type]}</Title>
        {txn.number && <Text c="dimmed">#{txn.number}</Text>}
        {txn.is_void && <Badge color="red">VOID</Badge>}
        <Badge variant="light" color="gray">
          Read-only: this form isn't built yet
        </Badge>
      </Group>
      <Card withBorder>
        <Group gap="xl">
          <div>
            <Text size="xs" c="dimmed">
              Date
            </Text>
            <Text>{formatDate(txn.date)}</Text>
          </div>
          {txn.entity_id && (
            <div>
              <Text size="xs" c="dimmed">
                Name
              </Text>
              <Text>{name(txn.entity_id)}</Text>
            </div>
          )}
          {txn.account_id && (
            <div>
              <Text size="xs" c="dimmed">
                Account
              </Text>
              <Text>{name(txn.account_id)}</Text>
            </div>
          )}
          {txn.to_account_id && (
            <div>
              <Text size="xs" c="dimmed">
                To account
              </Text>
              <Text>{name(txn.to_account_id)}</Text>
            </div>
          )}
          <div>
            <Text size="xs" c="dimmed">
              Amount
            </Text>
            <Text fw={700}>{formatMoney(txn.amount)}</Text>
          </div>
          {txn.memo && (
            <div>
              <Text size="xs" c="dimmed">
                Memo
              </Text>
              <Text>{txn.memo}</Text>
            </div>
          )}
        </Group>
      </Card>
      {txn.lines.length > 0 && (
        <Table.ScrollContainer minWidth={600}>
          <Table striped withTableBorder fz="sm">
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Account / Item</Table.Th>
                <Table.Th>Description</Table.Th>
                <Table.Th ta="right">Qty</Table.Th>
                <Table.Th ta="right">Rate</Table.Th>
                <Table.Th ta="right">Amount</Table.Th>
                <Table.Th>Name</Table.Th>
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {txn.lines.map((l) => (
                <Table.Tr key={l.id}>
                  <Table.Td>{name(l.item_id ?? l.account_id)}</Table.Td>
                  <Table.Td>{l.description}</Table.Td>
                  <Table.Td ta="right">{l.quantity}</Table.Td>
                  <Table.Td ta="right">{l.rate}</Table.Td>
                  <Table.Td ta="right" className={classes.num}>
                    {formatMoney(l.amount)}
                  </Table.Td>
                  <Table.Td>{name(l.entity_id)}</Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
        </Table.ScrollContainer>
      )}
      <Text fw={600}>Postings</Text>
      <Table withTableBorder fz="sm">
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Account</Table.Th>
            <Table.Th ta="right">Debit</Table.Th>
            <Table.Th ta="right">Credit</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {data.postings.map((p, i) => (
            <Table.Tr key={i}>
              <Table.Td>{name(p.account_id)}</Table.Td>
              <Table.Td ta="right">{p.amount > 0 ? formatMoney(p.amount) : ''}</Table.Td>
              <Table.Td ta="right">{p.amount < 0 ? formatMoney(-p.amount) : ''}</Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
    </Stack>
  );
}
