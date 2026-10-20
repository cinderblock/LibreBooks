// Home: what a bookkeeper checks first. Money in the bank and on the cards,
// what's owed both ways, the common forms one click away, and the latest
// transactions.

import { Badge, Card, Group, SimpleGrid, Skeleton, Stack, Table, Text, Title, UnstyledButton } from '@mantine/core';
import { IconBuildingBank, IconChecks, IconFileInvoice, IconReceipt, IconReportMoney, IconCash, IconListTree, IconCreditCard } from '@tabler/icons-react';
import type { ReactNode } from 'react';
import { useAccounts, useBalanceMap, useCompanyInfo, useTxns } from '~/api/hooks';
import { naturalBalance } from '~/lib/accountTypes';
import { formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useComboHint } from '~/keyboard/KeyboardProvider';
import type { CommandId } from '~/keyboard/commands';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

function Action({ icon, label, path, command, todo }: { icon: ReactNode; label: string; path: string; command?: CommandId; todo?: boolean }) {
  const open = useOpenWindow();
  const hint = useComboHint(command ?? 'palette.open');
  const mobile = useIsMobile();
  return (
    <UnstyledButton className={classes.action} onClick={() => open(path)} data-todo={todo || undefined}>
      <div className={classes.actionIcon}>{icon}</div>
      <Text size="sm" fw={500}>
        {label}
      </Text>
      {command && hint && !mobile && (
        <Text size="xs" c="dimmed">
          {hint}
        </Text>
      )}
    </UnstyledButton>
  );
}

export function Home() {
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const { data: info } = useCompanyInfo();
  const { data: accounts, isLoading } = useAccounts();
  const balances = useBalanceMap();
  const recent = useTxns({ descending: true, limit: 12 });
  const money = (accounts ?? []).filter((a) => a.is_active && (a.account_type === 'Bank' || a.account_type === 'CreditCard'));
  const total = (type: 'AccountsReceivable' | 'AccountsPayable') =>
    (accounts ?? []).filter((a) => a.account_type === type).reduce((s, a) => s + naturalBalance(a.account_type, balances.get(a.id) ?? 0), 0);

  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page}>
      {!mobile && <Title order={3}>{info?.name ?? 'Home'}</Title>}
      <SimpleGrid cols={{ base: 3, sm: 4, lg: 8 }} spacing="xs">
        <Action icon={<IconChecks size={22} />} label="Write Checks" path={paths.newCheck()} command="form.writeChecks" />
        <Action icon={<IconFileInvoice size={22} />} label="Invoice" path={paths.form('invoice')} command="form.invoice" todo />
        <Action icon={<IconCash size={22} />} label="Receive Payment" path={paths.form('receive-payment')} todo />
        <Action icon={<IconReceipt size={22} />} label="Enter Bills" path={paths.form('bill')} todo />
        <Action icon={<IconBuildingBank size={22} />} label="Deposit" path={paths.form('deposit')} todo />
        <Action icon={<IconCreditCard size={22} />} label="Card Charge" path={paths.form('cc-charge')} todo />
        <Action icon={<IconListTree size={22} />} label="Accounts" path={paths.accounts()} command="nav.chartOfAccounts" />
        <Action icon={<IconReportMoney size={22} />} label="Reports" path={paths.reports()} />
      </SimpleGrid>

      <SimpleGrid cols={{ base: 1, md: 2 }} spacing="md">
        <Card withBorder padding="sm">
          <Text fw={600} mb="xs">
            Bank and credit card accounts
          </Text>
          {isLoading ? (
            <Skeleton h={120} />
          ) : (
            <Table highlightOnHover verticalSpacing={6}>
              <Table.Tbody>
                {money.map((a) => {
                  const bal = naturalBalance(a.account_type, balances.get(a.id) ?? 0);
                  return (
                    <Table.Tr key={a.id} className={classes.clickable} onClick={() => open(paths.register(a.id))}>
                      <Table.Td>
                        <Text size="sm">{a.full_name}</Text>
                        <Text size="xs" c="dimmed">
                          {a.account_type === 'Bank' ? 'Bank' : 'Credit Card'}
                        </Text>
                      </Table.Td>
                      <Table.Td ta="right" className={classes.num} c={bal < 0 ? 'red' : undefined}>
                        {formatMoney(bal)}
                      </Table.Td>
                    </Table.Tr>
                  );
                })}
              </Table.Tbody>
            </Table>
          )}
        </Card>
        <Stack gap="md">
          <SimpleGrid cols={2} spacing="md">
            <Card withBorder padding="sm" className={classes.clickable} onClick={() => open(paths.report('customer_balance_summary'))}>
              <Text size="sm" c="dimmed">
                Customers owe you
              </Text>
              <Text size="xl" fw={700} className={classes.num}>
                {formatMoney(total('AccountsReceivable'), { symbol: true })}
              </Text>
            </Card>
            <Card withBorder padding="sm" className={classes.clickable} onClick={() => open(paths.vendors())}>
              <Text size="sm" c="dimmed">
                You owe vendors
              </Text>
              <Text size="xl" fw={700} className={classes.num}>
                {formatMoney(total('AccountsPayable'), { symbol: true })}
              </Text>
            </Card>
          </SimpleGrid>
          <Card withBorder padding="sm">
            <Group justify="space-between" mb="xs">
              <Text fw={600}>Recent transactions</Text>
              <Badge variant="light" className={classes.clickable} onClick={() => open(paths.report('transaction_list_by_date'))}>
                All
              </Badge>
            </Group>
            {recent.isLoading ? (
              <Skeleton h={160} />
            ) : (
              <Table highlightOnHover verticalSpacing={4} fz="sm">
                <Table.Tbody>
                  {(recent.data ?? []).map((t) => (
                    <Table.Tr key={t.id} className={classes.clickable} onClick={() => open(paths.txn(t.id))}>
                      <Table.Td w={70}>{formatDateShort(t.date)}</Table.Td>
                      <Table.Td>
                        <Text size="sm" truncate maw={mobile ? 150 : 260}>
                          {t.entity_name ?? t.memo ?? '—'}
                        </Text>
                        <Text size="xs" c="dimmed">
                          {TXN_LABEL[t.txn_type]}
                          {t.number ? ` ${t.number}` : ''}
                        </Text>
                      </Table.Td>
                      <Table.Td ta="right" className={classes.num}>
                        {formatMoney(t.amount)}
                      </Table.Td>
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            )}
          </Card>
        </Stack>
      </SimpleGrid>
    </Stack>
  );
}
