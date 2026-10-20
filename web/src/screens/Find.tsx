// Find (Ctrl+F): QBD's Simple Find. Filters run on the server (`GET /txns`).

import { Group, MultiSelect, Skeleton, Stack, Table, Text, TextInput, Title } from '@mantine/core';
import { useDebouncedValue } from '@mantine/hooks';
import { useState } from 'react';
import { useTxns } from '~/api/hooks';
import type { TxnType } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { NameQuickFill } from '~/inputs/pickers';
import { formatDateShort, type IsoDate } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL, TXN_TYPES } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

export function Find() {
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const [text, setText] = useState('');
  const [types, setTypes] = useState<string[]>([]);
  const [entity, setEntity] = useState<string | null>(null);
  const [from, setFrom] = useState<IsoDate | null>(null);
  const [to, setTo] = useState<IsoDate | null>(null);
  const [amount, setAmount] = useState<number | null>(null);
  const [number, setNumber] = useState('');
  const [debouncedText] = useDebouncedValue(text, 250);
  const any = debouncedText || types.length || entity || from || to || amount !== null || number;
  const results = useTxns(
    {
      text: debouncedText || null,
      types: types as TxnType[],
      entity_id: entity,
      include_jobs: true,
      from,
      to,
      amount,
      number: number || null,
      include_void: true,
      descending: true,
      limit: 500,
    },
    !!any,
  );
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="sm" className={classes.page}>
      {!mobile && <Title order={3}>Find</Title>}
      <Group gap="sm" align="flex-end" wrap="wrap">
        <TextInput label="Memo, number or name contains" value={text} onChange={(e) => setText(e.currentTarget.value)} w={260} autoFocus={!mobile} data-autofocus />
        <MultiSelect label="Transaction type" data={TXN_TYPES.map((t) => ({ value: t, label: TXN_LABEL[t] }))} value={types} onChange={setTypes} w={260} searchable clearable />
        <NameQuickFill label="Name" value={entity} onChange={setEntity} allowAddNew={false} />
        <DateField label="From" value={from} onChange={setFrom} />
        <DateField label="To" value={to} onChange={setTo} />
        <MoneyField label="Amount" value={amount} onChange={setAmount} />
        <TextInput label="Number" value={number} onChange={(e) => setNumber(e.currentTarget.value)} w={110} />
      </Group>
      {!any ? (
        <Text c="dimmed">Enter something to look for.</Text>
      ) : results.isLoading ? (
        <Skeleton h={240} />
      ) : (
        <>
          <Text size="sm" c="dimmed">
            {results.data?.length ?? 0} found
          </Text>
          <Table.ScrollContainer minWidth={640}>
            <Table highlightOnHover striped fz="sm" verticalSpacing={4}>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>Date</Table.Th>
                  <Table.Th>Type</Table.Th>
                  <Table.Th>Num</Table.Th>
                  <Table.Th>Name</Table.Th>
                  <Table.Th>Account</Table.Th>
                  <Table.Th>Memo</Table.Th>
                  <Table.Th ta="right">Amount</Table.Th>
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {(results.data ?? []).map((t) => (
                  <Table.Tr key={t.id} className={classes.clickable} onClick={() => open(paths.txn(t.id))}>
                    <Table.Td>{formatDateShort(t.date)}</Table.Td>
                    <Table.Td>{TXN_LABEL[t.txn_type]}</Table.Td>
                    <Table.Td>{t.number}</Table.Td>
                    <Table.Td>{t.entity_name}</Table.Td>
                    <Table.Td>{t.account_name}</Table.Td>
                    <Table.Td>{t.is_void ? 'VOID ' : ''}{t.memo}</Table.Td>
                    <Table.Td ta="right" className={classes.num}>
                      {formatMoney(t.amount)}
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          </Table.ScrollContainer>
        </>
      )}
    </Stack>
  );
}
