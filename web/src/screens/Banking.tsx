// Banking overview: every bank and card account with its register and balance,
// plus the banking forms.

import { Button, Card, Group, SimpleGrid, Stack, Text, Title } from '@mantine/core';
import { useAccounts, useBalanceMap } from '~/api/hooks';
import { naturalBalance } from '~/lib/accountTypes';
import { formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { FORM_LABELS } from '~/shell/menus';
import classes from './screens.module.css';

export function Banking() {
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const { data: accounts = [] } = useAccounts();
  const balances = useBalanceMap();
  const cash = accounts.filter((a) => a.is_active && ['Bank', 'CreditCard', 'OtherCurrentAsset'].includes(a.account_type) && !a.role);
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page}>
      {!mobile && <Title order={3}>Banking</Title>}
      <Group gap="xs">
        <Button onClick={() => open(paths.newCheck())}>Write Checks</Button>
        {(['deposit', 'transfer', 'cc-charge', 'reconcile'] as const).map((f) => (
          <Button key={f} variant="default" onClick={() => open(paths.form(f))}>
            {FORM_LABELS[f]}
          </Button>
        ))}
        <Button variant="default" onClick={() => open(paths.feeds())}>
          Bank Feeds
        </Button>
      </Group>
      <SimpleGrid cols={{ base: 1, sm: 2, lg: 3 }} spacing="sm">
        {cash.map((a) => (
          <Card key={a.id} withBorder className={classes.clickable} onClick={() => open(paths.register(a.id))} padding="sm">
            <Text fw={600}>{a.full_name}</Text>
            <Text size="xs" c="dimmed">
              {a.description ?? a.account_type}
            </Text>
            <Text size="xl" fw={700} className={classes.num} mt={4}>
              {formatMoney(naturalBalance(a.account_type, balances.get(a.id) ?? 0), { symbol: true })}
            </Text>
          </Card>
        ))}
      </SimpleGrid>
    </Stack>
  );
}
