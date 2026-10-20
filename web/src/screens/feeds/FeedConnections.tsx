// Connections: where downloads come from. A file connection remembers which
// register a bank's files belong to; SimpleFIN is claimed once with a setup
// token and then feeds one connection per bank account; Direct Connect (OFX)
// talks to the bank's own server. Credentials are write-only: set here, kept
// by the server, never shown.

import { Alert, Badge, Button, Checkbox, Group, Modal, NumberInput, PasswordInput, Select, SegmentedControl, Stack, Table, Text, TextInput } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconPlus } from '@tabler/icons-react';
import { useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex, useAccounts, useFeedConnections, useFeedInvalidate } from '~/api/hooks';
import type { FeedConnection, ProviderSettings, StatementAccount } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { AccountQuickFill } from '~/inputs/pickers';
import { formatDate } from '~/lib/dates';
import { useIsMobile } from '~/lib/useMobile';

const PROVIDER_LABEL: Record<ProviderSettings['provider'], string> = { File: 'File import', SimpleFin: 'SimpleFIN', OfxDirect: 'Direct Connect', Plaid: 'Plaid', Teller: 'Teller' };
const FEED_ACCOUNT_TYPES = ['Bank', 'CreditCard', 'OtherCurrentAsset', 'OtherCurrentLiability', 'LongTermLiability'] as const;

const blankConnection = (settings: ProviderSettings): FeedConnection => ({
  id: '',
  name: '',
  account_id: null,
  settings,
  account_key: null,
  reverse_signs: false,
  match_days: 10,
  not_before: null,
  last_sync_at: null,
  last_error: null,
  edit_seq: 0,
});

export function FeedConnections({ onImport }: { onImport(accountId: string | null): void }) {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const connections = useFeedConnections();
  const refresh = useFeedInvalidate();
  const accounts = useAccountIndex();
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState<FeedConnection | null>(null);
  const [credsFor, setCredsFor] = useState<FeedConnection | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const list = connections.data ?? [];
  const sync = async (c: FeedConnection) => {
    setBusy(c.id);
    try {
      const r = await api.syncFeedConnection(co, c.id);
      refresh();
      notifications.show({ message: `${c.name}: ${r.import.imported.length} new, ${r.matched.length} matched, ${r.auto_added.length} added, ${r.waiting} waiting.`, color: 'teal' });
    } catch (e) {
      refresh();
      notifications.show({ color: 'red', message: describeError(e) });
    } finally {
      setBusy(null);
    }
  };
  const remove = async (c: FeedConnection) => {
    if (!window.confirm(`Remove "${c.name}"? Its downloads stay; its credentials are deleted unless another connection uses them.`)) return;
    try {
      await api.deleteFeedConnection(co, c.id, c.edit_seq);
      refresh();
    } catch (e) {
      notifications.show({ color: 'red', message: describeError(e) });
    }
  };
  return (
    <Stack gap="sm">
      <Group justify="space-between" wrap="wrap">
        <Text size="sm" c="dimmed">
          Online connections download by themselves every few hours; files are imported by hand.
        </Text>
        <Button size="xs" leftSection={<IconPlus size={14} />} onClick={() => setCreating(true)}>
          New connection
        </Button>
      </Group>
      {connections.error && <Alert color="red">{describeError(connections.error)}</Alert>}
      <Table fz="sm" withTableBorder aria-label="Connections">
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Connection</Table.Th>
            <Table.Th>Register</Table.Th>
            {!mobile && <Table.Th>Source</Table.Th>}
            {!mobile && <Table.Th>Last download</Table.Th>}
            <Table.Th />
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {list.map((c) => (
            <Table.Tr key={c.id} data-testid="feed-connection">
              <Table.Td>
                <Text size="sm" fw={500}>
                  {c.name}
                </Text>
                {c.last_error && (
                  <Text size="xs" c="red">
                    {c.last_error}
                  </Text>
                )}
              </Table.Td>
              <Table.Td>{c.account_id ? (accounts.get(c.account_id)?.full_name ?? '?') : <Badge color="orange" size="xs">choose a register</Badge>}</Table.Td>
              {!mobile && <Table.Td>{PROVIDER_LABEL[c.settings.provider]}</Table.Td>}
              {!mobile && <Table.Td>{c.last_sync_at ? formatDate(c.last_sync_at.slice(0, 10)) : '—'}</Table.Td>}
              <Table.Td>
                <Group gap={4} wrap="nowrap" justify="flex-end">
                  {c.settings.provider === 'File' ? (
                    <Button size="compact-xs" variant="light" onClick={() => onImport(c.account_id)}>
                      Import file
                    </Button>
                  ) : (
                    <Button size="compact-xs" variant="light" loading={busy === c.id} onClick={() => void sync(c)}>
                      Download now
                    </Button>
                  )}
                  {c.settings.provider !== 'File' && (
                    <Button size="compact-xs" variant="subtle" onClick={() => setCredsFor(c)}>
                      Credentials
                    </Button>
                  )}
                  <Button size="compact-xs" variant="subtle" onClick={() => setEditing(c)}>
                    Edit
                  </Button>
                  <Button size="compact-xs" variant="subtle" color="red" onClick={() => void remove(c)}>
                    Remove
                  </Button>
                </Group>
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
      {creating && <NewConnectionModal onClose={() => setCreating(false)} />}
      {editing && <EditConnectionModal connection={editing} onClose={() => setEditing(null)} />}
      {credsFor && <CredentialsModal connection={credsFor} onClose={() => setCredsFor(null)} />}
    </Stack>
  );
}

function EditConnectionModal({ connection, onClose }: { connection: FeedConnection; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const [c, setC] = useState(connection);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await api.updateFeedConnection(co, c);
      refresh();
      onClose();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal opened onClose={onClose} title={`Edit ${connection.name}`} centered>
      <Stack gap="sm">
        <ConnectionFields c={c} onChange={setC} />
        {error && <Alert color="red">{error}</Alert>}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button loading={busy} onClick={() => void save()}>
            Save
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

function ConnectionFields({ c, onChange }: { c: FeedConnection; onChange(c: FeedConnection): void }) {
  return (
    <>
      <TextInput label="Name" value={c.name} onChange={(e) => onChange({ ...c, name: e.currentTarget.value })} required data-autofocus />
      <AccountQuickFill label="Register it feeds" types={FEED_ACCOUNT_TYPES} value={c.account_id} onChange={(account_id) => onChange({ ...c, account_id })} allowAddNew={false} aria-label="Register" />
      <Group grow>
        <NumberInput label="Match within (days)" value={c.match_days} onChange={(v) => onChange({ ...c, match_days: Number(v) || 10 })} min={0} max={60} />
        <DateField label="Ignore downloads before" value={c.not_before} onChange={(not_before) => onChange({ ...c, not_before })} />
      </Group>
      <Checkbox label="This source writes money in as negative (flip every amount)" checked={c.reverse_signs} onChange={(e) => onChange({ ...c, reverse_signs: e.currentTarget.checked })} />
    </>
  );
}

function CredentialsModal({ connection, onClose }: { connection: FeedConnection; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const [value, setValue] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const what = connection.settings.provider === 'SimpleFin' ? 'SimpleFIN access URL' : connection.settings.provider === 'OfxDirect' ? 'password or PIN' : 'access token';
  const save = async () => {
    setBusy(true);
    try {
      await api.setFeedCredentials(co, connection.id, value);
      refresh();
      notifications.show({ message: 'Credentials saved on the server.', color: 'teal' });
      onClose();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal opened onClose={onClose} title={`Credentials for ${connection.name}`} centered>
      <Stack gap="sm">
        <Text size="sm" c="dimmed">
          The {what} is kept in the server's secret store and never shown again. Entering it here replaces the old one.
        </Text>
        <PasswordInput label={`New ${what}`} value={value} onChange={(e) => setValue(e.currentTarget.value)} data-autofocus />
        {error && <Alert color="red">{error}</Alert>}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button loading={busy} onClick={() => void save()} disabled={!value.trim()}>
            Save
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

function NewConnectionModal({ onClose }: { onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const { data: accountList = [] } = useAccounts();
  const [kind, setKind] = useState<'file' | 'simplefin' | 'ofx'>('file');
  const [c, setC] = useState<FeedConnection>(() => blankConnection({ provider: 'File', csv_mapping: null }));
  const [token, setToken] = useState('');
  const [claimed, setClaimed] = useState<{ access: string; accounts: StatementAccount[] } | null>(null);
  const [picked, setPicked] = useState<Record<string, string | null>>({});
  const [ofx, setOfx] = useState({ url: '', org: '', fid: '', bank_id: '', account_number: '', account_kind: 'Checking' as 'Checking' | 'Savings' | 'CreditCard', user_id: '', password: '', app_id: 'QWIN', app_ver: '2700' });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const feedAccounts = accountList.filter((a) => a.is_active && (FEED_ACCOUNT_TYPES as readonly string[]).includes(a.account_type));

  const claim = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.simplefinClaim(co, token.trim());
      setClaimed({ access: r.access, accounts: r.accounts });
      if (r.errors.length) setError(r.errors.join(' '));
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      if (kind === 'file') {
        if (!c.name.trim() || !c.account_id) throw new Error('Name the connection and choose its register.');
        await api.createFeedConnection(co, c);
      } else if (kind === 'simplefin') {
        if (!claimed) throw new Error('Claim the setup token first.');
        const chosen = claimed.accounts.filter((a) => picked[a.remote_id ?? a.account_key]);
        if (!chosen.length) throw new Error('Pick at least one account and the register it feeds.');
        for (const a of chosen) {
          await api.createFeedConnection(co, {
            ...blankConnection({ provider: 'SimpleFin', access: claimed.access, remote_account_id: a.remote_id ?? a.account_key }),
            name: `${a.institution ?? 'SimpleFIN'} ${a.name ?? a.account_id_masked}`,
            account_id: picked[a.remote_id ?? a.account_key] ?? null,
            account_key: a.account_key || null,
          });
        }
      } else {
        if (!c.name.trim() || !c.account_id) throw new Error('Name the connection and choose its register.');
        if (!ofx.url || !ofx.org || !ofx.fid || !ofx.account_number || !ofx.user_id) throw new Error('The bank URL, ORG, FID, account number and user id are needed.');
        const saved = await api.createFeedConnection(co, {
          ...c,
          settings: { provider: 'OfxDirect', url: ofx.url, org: ofx.org, fid: ofx.fid, bank_id: ofx.bank_id || null, account_number: ofx.account_number, account_kind: ofx.account_kind, user_id: ofx.user_id, password: `secret:feeds.${co}.ofx-${crypto.randomUUID().slice(0, 8)}`, app_id: ofx.app_id, app_ver: ofx.app_ver, version: 'V220', client_uid: null, user_agent: null },
        });
        if (ofx.password) await api.setFeedCredentials(co, saved.id, ofx.password);
      }
      refresh();
      notifications.show({ message: 'Connection created.', color: 'teal' });
      onClose();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal opened onClose={onClose} title="New connection" size="lg" centered>
      <Stack gap="sm">
        <SegmentedControl
          data={[
            { value: 'file', label: 'Bank file (QBO/QFX/OFX/CSV)' },
            { value: 'simplefin', label: 'SimpleFIN' },
            { value: 'ofx', label: 'Direct Connect' },
          ]}
          value={kind}
          onChange={(v) => setKind(v as typeof kind)}
        />
        {kind === 'file' && (
          <>
            <Text size="sm" c="dimmed">
              Download Web Connect or CSV files from your bank and import them; the first import remembers the account, later files land there by themselves.
            </Text>
            <ConnectionFields c={c} onChange={setC} />
          </>
        )}
        {kind === 'simplefin' && (
          <>
            <Text size="sm" c="dimmed">
              Get a setup token from your SimpleFIN bridge (beta-bridge.simplefin.org), paste it once; the server exchanges it for an access URL it keeps.
            </Text>
            <Group align="flex-end">
              <TextInput label="Setup token" value={token} onChange={(e) => setToken(e.currentTarget.value)} style={{ flex: 1 }} data-autofocus />
              <Button loading={busy} onClick={() => void claim()} disabled={!token.trim()}>
                Claim
              </Button>
            </Group>
            {claimed && (
              <Table fz="sm">
                <Table.Thead>
                  <Table.Tr>
                    <Table.Th>Bank account</Table.Th>
                    <Table.Th>Feeds register</Table.Th>
                  </Table.Tr>
                </Table.Thead>
                <Table.Tbody>
                  {claimed.accounts.map((a) => {
                    const key = a.remote_id ?? a.account_key;
                    return (
                      <Table.Tr key={key}>
                        <Table.Td>
                          {a.institution} {a.name ?? ''} {a.account_id_masked}
                        </Table.Td>
                        <Table.Td>
                          <Select size="xs" placeholder="(skip)" data={feedAccounts.map((x) => ({ value: x.id, label: x.full_name }))} value={picked[key] ?? null} onChange={(v) => setPicked({ ...picked, [key]: v })} clearable searchable />
                        </Table.Td>
                      </Table.Tr>
                    );
                  })}
                </Table.Tbody>
              </Table>
            )}
          </>
        )}
        {kind === 'ofx' && (
          <>
            <ConnectionFields c={c} onChange={setC} />
            <Group grow>
              <TextInput label="OFX server URL" value={ofx.url} onChange={(e) => setOfx({ ...ofx, url: e.currentTarget.value })} placeholder="https://ofx.bank.com/ofx" />
              <TextInput label="ORG" value={ofx.org} onChange={(e) => setOfx({ ...ofx, org: e.currentTarget.value })} />
              <TextInput label="FID" value={ofx.fid} onChange={(e) => setOfx({ ...ofx, fid: e.currentTarget.value })} />
            </Group>
            <Group grow>
              <TextInput label="Routing number" value={ofx.bank_id} onChange={(e) => setOfx({ ...ofx, bank_id: e.currentTarget.value })} />
              <TextInput label="Account number" value={ofx.account_number} onChange={(e) => setOfx({ ...ofx, account_number: e.currentTarget.value })} />
              <Select label="Kind" data={['Checking', 'Savings', 'CreditCard']} value={ofx.account_kind} onChange={(v) => v && setOfx({ ...ofx, account_kind: v as typeof ofx.account_kind })} allowDeselect={false} />
            </Group>
            <Group grow>
              <TextInput label="User id" value={ofx.user_id} onChange={(e) => setOfx({ ...ofx, user_id: e.currentTarget.value })} />
              <PasswordInput label="Password / PIN (write-only)" value={ofx.password} onChange={(e) => setOfx({ ...ofx, password: e.currentTarget.value })} />
              <TextInput label="App id / version" value={`${ofx.app_id} ${ofx.app_ver}`} onChange={(e) => { const [app_id = 'QWIN', app_ver = '2700'] = e.currentTarget.value.split(' '); setOfx({ ...ofx, app_id, app_ver }); }} />
            </Group>
          </>
        )}
        {error && <Alert color="red">{error}</Alert>}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button loading={busy} onClick={() => void create()}>
            Create
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
