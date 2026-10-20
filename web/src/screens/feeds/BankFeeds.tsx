// Bank Feeds (Banking → Bank Feeds): what each account has downloaded and
// waiting, the queue of downloads to match, add or ignore, the rules that
// suggest how, the connections that download, file import and Amazon order
// matching. Routes: plans/api.md "Bank feeds".

import { Badge, Button, Group, Stack, Table, Tabs, Text, Title } from '@mantine/core';
import { IconFileImport, IconPlugConnected, IconRefresh } from '@tabler/icons-react';
import { useEffect, useState } from 'react';
import { useAccountIndex, useFeedsSummary } from '~/api/hooks';
import { formatDate, formatDateShort } from '~/lib/dates';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow } from '~/windows/WindowsProvider';
import classes from '../screens.module.css';
import { FeedConnections } from './FeedConnections';
import { AmazonPanel, ImportModal } from './FeedImport';
import { FeedQueue } from './FeedQueue';
import { FeedRules } from './FeedRules';

type Tab = 'accounts' | 'queue' | 'rules' | 'connections' | 'amazon';

export function BankFeeds() {
  const win = useWindow();
  const mobile = useIsMobile();
  const [tab, setTab] = useState<Tab>((win.desc.params.tab as Tab | undefined) ?? 'accounts');
  const [queueAccount, setQueueAccount] = useState<string | null>(win.desc.params.account ?? null);
  const [importFor, setImportFor] = useState<string | null | false>(false);
  useEffect(() => win.setTitle('Bank Feeds'), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="bank-feeds">
      <Tabs value={tab} onChange={(v) => setTab((v as Tab) ?? 'accounts')} keepMounted={false} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
        <Tabs.List px={mobile ? 'sm' : 0}>
          <Tabs.Tab value="accounts">Accounts</Tabs.Tab>
          <Tabs.Tab value="queue">Downloaded</Tabs.Tab>
          <Tabs.Tab value="rules">Rules</Tabs.Tab>
          <Tabs.Tab value="connections">Connections</Tabs.Tab>
          <Tabs.Tab value="amazon">Amazon</Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel value="accounts" pt="sm" px={mobile ? 'sm' : 0}>
          <AccountsPanel
            onReview={(id) => {
              setQueueAccount(id);
              setTab('queue');
            }}
            onImport={(id) => setImportFor(id)}
            onConnect={() => setTab('connections')}
          />
        </Tabs.Panel>
        <Tabs.Panel value="queue" pt="sm" px={mobile ? 'sm' : 0} style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
          <FeedQueue accountId={queueAccount} onAccountChange={setQueueAccount} />
        </Tabs.Panel>
        <Tabs.Panel value="rules" pt="sm" px={mobile ? 'sm' : 0}>
          <FeedRules />
        </Tabs.Panel>
        <Tabs.Panel value="connections" pt="sm" px={mobile ? 'sm' : 0}>
          <FeedConnections onImport={(id) => setImportFor(id)} />
        </Tabs.Panel>
        <Tabs.Panel value="amazon" pt="sm" px={mobile ? 'sm' : 0}>
          <AmazonPanel />
        </Tabs.Panel>
      </Tabs>
      <ImportModal opened={importFor !== false} accountId={importFor === false ? null : importFor} onClose={() => setImportFor(false)} />
    </div>
  );
}

function AccountsPanel({ onReview, onImport, onConnect }: { onReview(accountId: string): void; onImport(accountId: string | null): void; onConnect(): void }) {
  const mobile = useIsMobile();
  const accounts = useAccountIndex();
  const summary = useFeedsSummary();
  const rows = (summary.data ?? []).map((s) => ({ ...s, name: accounts.get(s.account_id)?.full_name ?? s.account_id }));
  return (
    <Stack gap="sm">
      <Group justify="space-between" wrap="wrap">
        {!mobile && <Title order={4}>Accounts with downloads</Title>}
        <Group gap="xs">
          <Button size="xs" leftSection={<IconFileImport size={14} />} onClick={() => onImport(null)}>
            Import a file
          </Button>
          <Button size="xs" variant="default" leftSection={<IconPlugConnected size={14} />} onClick={onConnect}>
            Connect an account
          </Button>
        </Group>
      </Group>
      {rows.length === 0 && !summary.isLoading && (
        <Text c="dimmed" size="sm">
          Nothing downloaded yet. Import a Web Connect file (.qbo/.qfx/.ofx) or a CSV from your bank, or connect an account.
        </Text>
      )}
      <Table fz="sm" withTableBorder highlightOnHover>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Account</Table.Th>
            <Table.Th ta="right">New</Table.Th>
            {!mobile && <Table.Th ta="right">Matched</Table.Th>}
            {!mobile && <Table.Th ta="right">Added</Table.Th>}
            {!mobile && <Table.Th ta="right">Ignored</Table.Th>}
            <Table.Th>Latest</Table.Th>
            {!mobile && <Table.Th>Last import</Table.Th>}
            <Table.Th />
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {rows.map((r) => (
            <Table.Tr key={r.account_id} data-testid="feed-account-row">
              <Table.Td>{r.name}</Table.Td>
              <Table.Td ta="right">{r.new ? <Badge color="blue">{r.new}</Badge> : '0'}</Table.Td>
              {!mobile && <Table.Td ta="right">{r.matched}</Table.Td>}
              {!mobile && <Table.Td ta="right">{r.added}</Table.Td>}
              {!mobile && <Table.Td ta="right">{r.ignored}</Table.Td>}
              <Table.Td>{r.latest_posted ? formatDateShort(r.latest_posted) : '—'}</Table.Td>
              {!mobile && <Table.Td>{r.last_imported_at ? formatDate(r.last_imported_at.slice(0, 10)) : '—'}</Table.Td>}
              <Table.Td>
                <Group gap={4} wrap="nowrap" justify="flex-end">
                  <Button size="compact-xs" variant="light" onClick={() => onReview(r.account_id)}>
                    Review
                  </Button>
                  <Button size="compact-xs" variant="subtle" leftSection={<IconRefresh size={12} />} onClick={() => onImport(r.account_id)}>
                    Import
                  </Button>
                </Group>
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
    </Stack>
  );
}
