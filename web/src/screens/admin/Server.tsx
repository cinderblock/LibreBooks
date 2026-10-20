// Server settings (Company → Server Settings; admin): the settings document
// (`PUT /server` sends it whole), the updater, the Cloudflare tunnel, the
// backup schedule, and the write-only secrets. Problems the server finds come
// back as a list (400 `invalid`, `detail.problems`) and are shown as such.

import { ActionIcon, Alert, Badge, Button, Card, Code, Group, List, Modal, NumberInput, PasswordInput, Select, SimpleGrid, Stack, Switch, Table, Tabs, Text, Textarea, TextInput, Title, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconCloud, IconDownload, IconKey, IconRefresh, IconTrash } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { useApi } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import { useInvalidate, useServer, useTunnel, useUpdate } from '~/api/hooks';
import { sk } from '~/api/keys';
import type { BackupConfig, BackupDestination, BackupEncryption, ServerSettings, TunnelUpdate, TunnelView } from '~/api/types';
import { formatDateTime } from '~/lib/dates';
import { describeDuration, durationToMinutes, minutesToDuration } from '~/lib/format';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow } from '~/windows/WindowsProvider';
import classes from '../screens.module.css';
import { AdminGate, Confirm, When } from './common';

type Tab = 'general' | 'updates' | 'tunnel' | 'backups' | 'secrets';

export function ServerWindow() {
  const win = useWindow();
  useEffect(() => win.setTitle('Server Settings'), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <AdminGate what="Changing server settings">
      <Server />
    </AdminGate>
  );
}

/** The server's "fix these first" list, from a 400 `invalid`. */
function problemsOf(e: unknown): string[] {
  if (!isApiError(e)) return [];
  const d = e.detail as { problems?: unknown } | undefined;
  return Array.isArray(d?.problems) ? d.problems.map(String) : [];
}

function ErrorBox({ error }: { error: unknown }) {
  if (!error) return null;
  const problems = problemsOf(error);
  return (
    <Alert color="red" title={problems.length ? 'Fix these first' : undefined}>
      {problems.length ? (
        <List size="sm">
          {problems.map((p) => (
            <List.Item key={p}>{p}</List.Item>
          ))}
        </List>
      ) : (
        describeError(error)
      )}
    </Alert>
  );
}

function Server() {
  const win = useWindow();
  const mobile = useIsMobile();
  const server = useServer();
  const [tab, setTab] = useState<Tab>((win.desc.params.tab as Tab | undefined) ?? 'general');
  const view = server.data;
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="server-settings">
      <Group justify="space-between" align="flex-start">
        <div>
          <Title order={3}>Server settings</Title>
          {view && (
            <Text size="sm" c="dimmed">
              LibreBooks {view.version} · data in <Code>{view.data_dir}</Code>
            </Text>
          )}
        </div>
        {view && view.restart_required.length > 0 && (
          <Alert color="yellow" py={6}>
            Restart the server for: {view.restart_required.join(', ')}
          </Alert>
        )}
      </Group>
      {server.error && <ErrorBox error={server.error} />}
      {view && (
        <Tabs value={tab} onChange={(v) => setTab((v as Tab) ?? 'general')} keepMounted={false}>
          <Tabs.List>
            <Tabs.Tab value="general">General</Tabs.Tab>
            <Tabs.Tab value="updates">Updates</Tabs.Tab>
            <Tabs.Tab value="tunnel">Tunnel</Tabs.Tab>
            <Tabs.Tab value="backups">Backups</Tabs.Tab>
            <Tabs.Tab value="secrets">Secrets</Tabs.Tab>
          </Tabs.List>
          <Tabs.Panel value="general" pt="md">
            <GeneralPanel settings={view.settings} />
          </Tabs.Panel>
          <Tabs.Panel value="updates" pt="md">
            <UpdatesPanel settings={view.settings} />
          </Tabs.Panel>
          <Tabs.Panel value="tunnel" pt="md">
            <TunnelPanel />
          </Tabs.Panel>
          <Tabs.Panel value="backups" pt="md">
            <BackupsPanel settings={view.settings} secrets={view.secrets.map((s) => s.name)} companies={view.backups} />
          </Tabs.Panel>
          <Tabs.Panel value="secrets" pt="md">
            <SecretsPanel secrets={view.secrets} />
          </Tabs.Panel>
        </Tabs>
      )}
    </Stack>
  );
}

/** Save the whole settings document with one part replaced. */
function useSaveSettings() {
  const api = useApi();
  const invalidate = useInvalidate();
  return useMutation({
    mutationFn: (settings: ServerSettings) => api.putServer(settings),
    onSuccess: (view) => {
      invalidate(sk.server);
      notifications.show({ message: view.restart_required.length ? `Saved. Restart needed for ${view.restart_required.join(', ')}.` : 'Server settings saved.', color: 'teal' });
    },
  });
}

function GeneralPanel({ settings }: { settings: ServerSettings }) {
  const save = useSaveSettings();
  const [urls, setUrls] = useState(settings.public_urls.join('\n'));
  const [feedsAuto, setFeedsAuto] = useState(settings.feeds.auto_sync);
  const [feedsEvery, setFeedsEvery] = useState(durationToMinutes(settings.feeds.every) ?? 720);
  const [access, setAccess] = useState(!!settings.access);
  const [team, setTeam] = useState(settings.access?.team_domain ?? '');
  const [aud, setAud] = useState(settings.access?.audience.join('\n') ?? '');
  const submit = () =>
    save.mutate({
      ...settings,
      public_urls: urls
        .split(/\s+/)
        .map((u) => u.trim())
        .filter(Boolean),
      feeds: { auto_sync: feedsAuto, every: minutesToDuration(feedsEvery) },
      access: access
        ? {
            team_domain: team.trim(),
            audience: aud
              .split(/\s+/)
              .map((a) => a.trim())
              .filter(Boolean),
          }
        : null,
    });
  return (
    <Stack gap="md" maw={640}>
      <Textarea
        label="Public URLs"
        description="Where people reach this server (one per line). Passkeys are registered against these, so changing them needs a restart and may need re-enrolment."
        value={urls}
        onChange={(e) => setUrls(e.currentTarget.value)}
        autosize
        minRows={2}
        aria-label="Public URLs"
      />
      <Card withBorder>
        <Text fw={600} mb="xs">
          Bank feeds
        </Text>
        <Group align="flex-end">
          <Switch label="Download from connected banks on a schedule" checked={feedsAuto} onChange={(e) => setFeedsAuto(e.currentTarget.checked)} />
          <NumberInput label="Every (minutes)" value={feedsEvery} onChange={(v) => setFeedsEvery(Number(v) || 60)} min={15} step={15} disabled={!feedsAuto} w={160} description={describeDuration(minutesToDuration(feedsEvery))} />
        </Group>
      </Card>
      <Card withBorder>
        <Switch label="Require Cloudflare Access" description="Requests through Cloudflare must carry a valid Access token; LAN and localhost are not affected." checked={access} onChange={(e) => setAccess(e.currentTarget.checked)} mb="xs" />
        {access && (
          <SimpleGrid cols={{ base: 1, sm: 2 }}>
            <TextInput label="Team domain" placeholder="myteam.cloudflareaccess.com" value={team} onChange={(e) => setTeam(e.currentTarget.value)} />
            <Textarea label="Application audience tags" placeholder="one per line" value={aud} onChange={(e) => setAud(e.currentTarget.value)} autosize minRows={1} />
          </SimpleGrid>
        )}
      </Card>
      <ErrorBox error={save.error} />
      <Group justify="flex-end">
        <Button onClick={submit} loading={save.isPending}>
          Save
        </Button>
      </Group>
    </Stack>
  );
}

function UpdatesPanel({ settings }: { settings: ServerSettings }) {
  const api = useApi();
  const invalidate = useInvalidate();
  const save = useSaveSettings();
  const update = useUpdate();
  const [cfg, setCfg] = useState(settings.updates);
  const [confirmInstall, setConfirmInstall] = useState(false);
  const check = useMutation({
    mutationFn: () => api.checkUpdate(),
    onSuccess: (v) => {
      invalidate(sk.server);
      notifications.show({ message: v.status?.available ? `Version ${v.status.available.version} is available.` : `You're on the latest version.`, color: 'teal' });
    },
  });
  const install = useMutation({
    mutationFn: (restart: boolean) => api.installUpdate(restart),
    onSuccess: (r) => {
      invalidate(sk.server);
      setConfirmInstall(false);
      notifications.show({ message: r.restarting ? 'Installed; the server is restarting. Reload in a moment.' : 'Installed; it starts on the next restart.', color: 'teal', autoClose: 10_000 });
    },
  });
  const view = update.data;
  const st = view?.status;
  return (
    <Stack gap="md" maw={640}>
      <Card withBorder>
        {view?.unavailable ? (
          <Alert color="gray">No updater: {view.unavailable}</Alert>
        ) : st ? (
          <Stack gap="xs">
            <Group justify="space-between">
              <div>
                <Text fw={600}>Version {st.current}</Text>
                <Text size="xs" c="dimmed">
                  last checked <When iso={st.last_check} fallback="never" />
                </Text>
              </div>
              <Button variant="default" size="xs" leftSection={<IconRefresh size={14} />} loading={check.isPending} onClick={() => check.mutate()}>
                Check now
              </Button>
            </Group>
            {st.available ? (
              <Alert color="teal" title={`Version ${st.available.version} is available`} data-testid="update-available">
                <Group justify="space-between" align="center">
                  <Text size="sm">
                    <a href={st.available.page} target="_blank" rel="noreferrer">
                      Release notes
                    </a>
                    {' · '}
                    {st.available.asset_name}
                  </Text>
                  <Button size="xs" leftSection={<IconDownload size={14} />} disabled={!st.can_install} onClick={() => setConfirmInstall(true)}>
                    Install
                  </Button>
                </Group>
                {!st.can_install && st.advice && (
                  <Text size="xs" c="dimmed" mt={4}>
                    {st.advice}
                  </Text>
                )}
              </Alert>
            ) : (
              <Text size="sm" c="dimmed">
                {typeof st.phase === 'string' && st.phase !== 'Idle' ? st.phase : 'Up to date.'}
                {st.advice ? ` ${st.advice}` : ''}
              </Text>
            )}
          </Stack>
        ) : (
          <Text c="dimmed">Loading…</Text>
        )}
        <ErrorBox error={check.error} />
      </Card>
      <Card withBorder>
        <Stack gap="sm">
          <Switch label="Check for updates" checked={cfg.check} onChange={(e) => setCfg({ ...cfg, check: e.currentTarget.checked })} />
          <Switch label="Install updates automatically (restarts the server when one lands)" checked={cfg.auto_install} disabled={!cfg.check} onChange={(e) => setCfg({ ...cfg, auto_install: e.currentTarget.checked })} />
          <Group>
            <Select label="Channel" data={['Stable', 'Prerelease']} value={cfg.channel} onChange={(v) => v && setCfg({ ...cfg, channel: v as 'Stable' | 'Prerelease' })} allowDeselect={false} w={160} />
            <NumberInput label="Check every (hours)" value={(durationToMinutes(cfg.interval) ?? 360) / 60} onChange={(v) => setCfg({ ...cfg, interval: minutesToDuration((Number(v) || 6) * 60) })} min={1} w={160} disabled={!cfg.check} />
          </Group>
          <ErrorBox error={save.error} />
          <Group justify="flex-end">
            <Button onClick={() => save.mutate({ ...settings, updates: cfg })} loading={save.isPending}>
              Save
            </Button>
          </Group>
        </Stack>
      </Card>
      <Confirm opened={confirmInstall} title={`Install version ${st?.available?.version}?`} confirmLabel="Install and restart" color="teal" loading={install.isPending} onConfirm={() => install.mutate(true)} onClose={() => setConfirmInstall(false)}>
        <Text size="sm">The server downloads the release, checks its signature, and restarts into it. Everyone is disconnected for a few seconds; nothing in the books is touched.</Text>
        <Button variant="subtle" size="xs" mt="sm" onClick={() => install.mutate(false)} loading={install.isPending}>
          Install but don't restart yet
        </Button>
        <ErrorBox error={install.error} />
      </Confirm>
    </Stack>
  );
}

const TUNNEL_COLOR: Record<TunnelView['status']['state']['state'], string> = { stopped: 'gray', starting: 'yellow', connected: 'teal', reconnecting: 'yellow', failed: 'red' };

function TunnelPanel() {
  const api = useApi();
  const invalidate = useInvalidate();
  const tunnel = useTunnel();
  const [hostname, setHostname] = useState('');
  const [token, setToken] = useState('');
  const [path, setPath] = useState('');
  const [auto, setAuto] = useState(true);
  const t = tunnel.data;
  useEffect(() => {
    if (!t) return;
    setHostname(t.public_hostname ?? '');
    setPath(t.cloudflared_path ?? '');
    setAuto(t.auto_download);
  }, [t]);
  const put = useMutation({
    mutationFn: (u: TunnelUpdate) => api.putTunnel(u),
    onSuccess: (v) => {
      invalidate(sk.server);
      setToken('');
      notifications.show({ message: v.enabled ? 'Tunnel settings saved; cloudflared is connecting.' : 'Tunnel settings saved.', color: 'teal' });
    },
  });
  if (!t) return <ErrorBox error={tunnel.error} />;
  const state = t.status.state;
  return (
    <Stack gap="md" maw={640}>
      <Card withBorder>
        <Group justify="space-between" align="flex-start">
          <Group gap="xs">
            <IconCloud size={18} />
            <div>
              <Group gap="xs">
                <Text fw={600}>Cloudflare Tunnel</Text>
                <Badge color={TUNNEL_COLOR[state.state]} variant="light" data-testid="tunnel-state">
                  {state.state}
                  {state.state === 'connected' ? ` (${state.connections})` : ''}
                </Badge>
              </Group>
              <Text size="xs" c="dimmed">
                {t.public_url ? (
                  <a href={t.public_url} target="_blank" rel="noreferrer">
                    {t.public_url}
                  </a>
                ) : (
                  'Reaches this server from the internet without opening a port.'
                )}
                {t.cloudflared ? ` · cloudflared ${t.cloudflared.version} (${t.cloudflared.located.origin})` : ''}
              </Text>
            </div>
          </Group>
          <Switch label={t.enabled ? 'On' : 'Off'} checked={t.enabled} onChange={(e) => put.mutate({ enabled: e.currentTarget.checked })} disabled={put.isPending || (!t.token_set && !t.enabled)} />
        </Group>
        {state.state === 'failed' && (
          <Alert color="red" mt="sm">
            {state.reason}
          </Alert>
        )}
        {t.problems.length > 0 && (
          <Alert color="yellow" mt="sm">
            <List size="sm">
              {t.problems.map((p) => (
                <List.Item key={p}>{p}</List.Item>
              ))}
            </List>
          </Alert>
        )}
        {t.status.last_log_lines.length > 0 && (
          <Code block mt="sm" fz="xs">
            {t.status.last_log_lines.join('\n')}
          </Code>
        )}
      </Card>
      <Card withBorder>
        <Stack gap="sm">
          <PasswordInput
            label="Tunnel token"
            description={t.token_set ? 'A token is set. Paste a new one to replace it (the whole "cloudflared service install …" line works too).' : 'From the Cloudflare dashboard: Zero Trust → Networks → Tunnels → your tunnel.'}
            value={token}
            onChange={(e) => setToken(e.currentTarget.value)}
            placeholder={t.token_set ? '•••••••• (set)' : 'eyJh…'}
          />
          <TextInput label="Public hostname" placeholder="books.example.com" description="The hostname routed to this tunnel. Passkeys are registered against it, so changing it needs a restart." value={hostname} onChange={(e) => setHostname(e.currentTarget.value)} />
          <Switch label="Download cloudflared automatically" checked={auto} onChange={(e) => setAuto(e.currentTarget.checked)} />
          {!auto && <TextInput label="cloudflared path" placeholder="/usr/local/bin/cloudflared" value={path} onChange={(e) => setPath(e.currentTarget.value)} />}
          <ErrorBox error={put.error} />
          <Group justify="flex-end">
            <Button
              loading={put.isPending}
              onClick={() => put.mutate({ ...(token ? { token } : {}), public_hostname: hostname.trim(), auto_download: auto, cloudflared_path: auto ? '' : path.trim() })}
            >
              Save
            </Button>
          </Group>
        </Stack>
      </Card>
    </Stack>
  );
}

function destinationLabel(d: BackupDestination): string {
  return 'Local' in d ? `Folder: ${d.Local.dir}` : `S3: ${d.S3.bucket}/${d.S3.prefix} (${d.S3.endpoint || d.S3.region})`;
}

function BackupsPanel({ settings, secrets, companies }: { settings: ServerSettings; secrets: string[]; companies: Array<{ company_id: string; company_name: string; status: { next_run: string | null; last_success_at: string | null; last: { error: string | null } | null } }> }) {
  const save = useSaveSettings();
  const [cfg, setCfg] = useState<BackupConfig>(settings.backups);
  const [newDest, setNewDest] = useState<'Local' | 'S3' | null>(null);
  const [dir, setDir] = useState('');
  const [s3, setS3] = useState({ endpoint: '', region: 'us-east-1', bucket: '', prefix: 'librebooks', access_key_id: '', secret_ref: 's3-secret-key', path_style: false });
  const sched = cfg.schedule;
  const mode: 'daily' | 'every' = sched.every ? 'every' : 'daily';
  const enc = cfg.encryption;
  const encKind = enc === 'None' ? 'None' : 'Recipients' in enc ? 'Recipients' : 'Passphrase';
  const setEnc = (e: BackupEncryption) => setCfg({ ...cfg, encryption: e });
  const addDest = () => {
    const d: BackupDestination = newDest === 'Local' ? { Local: { dir: dir.trim() } } : { S3: { endpoint: s3.endpoint.trim(), region: s3.region.trim(), bucket: s3.bucket.trim(), prefix: s3.prefix.trim(), access_key_id: s3.access_key_id.trim(), secret_access_key_ref: s3.secret_ref.trim(), path_style: s3.path_style } };
    setCfg({ ...cfg, destinations: [...cfg.destinations, d] });
    setNewDest(null);
    setDir('');
  };
  return (
    <Stack gap="md" maw={720}>
      {companies.length > 0 && (
        <Table fz="sm" withTableBorder>
          <Table.Thead>
            <Table.Tr>
              <Table.Th>Company</Table.Th>
              <Table.Th>Last good backup</Table.Th>
              <Table.Th>Next</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {companies.map((c) => (
              <Table.Tr key={c.company_id}>
                <Table.Td>{c.company_name}</Table.Td>
                <Table.Td>
                  <When iso={c.status.last_success_at} fallback="never" />
                  {c.status.last?.error && (
                    <Text span c="red" size="xs">
                      {' '}
                      · last run failed: {c.status.last.error}
                    </Text>
                  )}
                </Table.Td>
                <Table.Td>{c.status.next_run ? formatDateTime(c.status.next_run) : '—'}</Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      )}
      <Card withBorder>
        <Stack gap="sm">
          <Switch label="Back up on a schedule" checked={sched.enabled} onChange={(e) => setCfg({ ...cfg, schedule: { ...sched, enabled: e.currentTarget.checked } })} />
          <Group align="flex-end">
            <Select
              label="When"
              data={[
                { value: 'daily', label: 'Daily at a time' },
                { value: 'every', label: 'Every N hours' },
              ]}
              value={mode}
              onChange={(v) => setCfg({ ...cfg, schedule: v === 'every' ? { ...sched, every: 'PT6H', daily_at: null } : { ...sched, every: null, daily_at: '02:30:00' } })}
              allowDeselect={false}
              w={180}
              disabled={!sched.enabled}
            />
            {mode === 'daily' ? (
              <TextInput label="Time (local)" value={(sched.daily_at ?? '02:30:00').slice(0, 5)} onChange={(e) => setCfg({ ...cfg, schedule: { ...sched, daily_at: `${e.currentTarget.value}:00` } })} type="time" w={140} disabled={!sched.enabled} />
            ) : (
              <NumberInput label="Hours" value={(durationToMinutes(sched.every) ?? 360) / 60} onChange={(v) => setCfg({ ...cfg, schedule: { ...sched, every: minutesToDuration((Number(v) || 6) * 60) } })} min={1} w={120} disabled={!sched.enabled} />
            )}
            <TextInput label="Time zone" placeholder="(server's)" value={cfg.time_zone ?? ''} onChange={(e) => setCfg({ ...cfg, time_zone: e.currentTarget.value.trim() || null })} w={200} />
          </Group>
        </Stack>
      </Card>
      <Card withBorder>
        <Text fw={600} mb="xs">
          Destinations
        </Text>
        {cfg.destinations.length === 0 && (
          <Text size="sm" c="dimmed" mb="xs">
            None yet; backups need somewhere to go.
          </Text>
        )}
        <Stack gap={4} mb="sm">
          {cfg.destinations.map((d, i) => (
            <Group key={i} justify="space-between" wrap="nowrap">
              <Text size="sm">{destinationLabel(d)}</Text>
              <ActionIcon variant="subtle" color="red" aria-label={`Remove destination ${i + 1}`} onClick={() => setCfg({ ...cfg, destinations: cfg.destinations.filter((_, j) => j !== i) })}>
                <IconTrash size={16} />
              </ActionIcon>
            </Group>
          ))}
        </Stack>
        {newDest === null ? (
          <Group gap="xs">
            <Button size="xs" variant="default" onClick={() => setNewDest('Local')}>
              Add a folder
            </Button>
            <Button size="xs" variant="default" onClick={() => setNewDest('S3')}>
              Add S3 storage
            </Button>
          </Group>
        ) : newDest === 'Local' ? (
          <Group align="flex-end">
            <TextInput label="Folder on the server" placeholder="/mnt/backups/librebooks" value={dir} onChange={(e) => setDir(e.currentTarget.value)} style={{ flex: 1 }} data-autofocus />
            <Button size="sm" onClick={addDest} disabled={!dir.trim()}>
              Add
            </Button>
            <Button size="sm" variant="subtle" onClick={() => setNewDest(null)}>
              Cancel
            </Button>
          </Group>
        ) : (
          <Stack gap="xs">
            <SimpleGrid cols={{ base: 1, sm: 2 }}>
              <TextInput label="Bucket" value={s3.bucket} onChange={(e) => setS3({ ...s3, bucket: e.currentTarget.value })} data-autofocus />
              <TextInput label="Prefix" value={s3.prefix} onChange={(e) => setS3({ ...s3, prefix: e.currentTarget.value })} />
              <TextInput label="Endpoint (blank for AWS)" placeholder="https://s3.us-west-004.backblazeb2.com" value={s3.endpoint} onChange={(e) => setS3({ ...s3, endpoint: e.currentTarget.value })} />
              <TextInput label="Region" value={s3.region} onChange={(e) => setS3({ ...s3, region: e.currentTarget.value })} />
              <TextInput label="Access key id" value={s3.access_key_id} onChange={(e) => setS3({ ...s3, access_key_id: e.currentTarget.value })} />
              <Select label="Secret key (a secret's name)" data={secrets} value={secrets.includes(s3.secret_ref) ? s3.secret_ref : null} onChange={(v) => v && setS3({ ...s3, secret_ref: v })} placeholder="add it on the Secrets tab" />
            </SimpleGrid>
            <Switch label="Path-style URLs (MinIO and some S3-compatibles)" checked={s3.path_style} onChange={(e) => setS3({ ...s3, path_style: e.currentTarget.checked })} />
            <Group>
              <Button size="sm" onClick={addDest} disabled={!s3.bucket.trim() || !s3.access_key_id.trim()}>
                Add
              </Button>
              <Button size="sm" variant="subtle" onClick={() => setNewDest(null)}>
                Cancel
              </Button>
            </Group>
          </Stack>
        )}
      </Card>
      <Card withBorder>
        <Text fw={600} mb="xs">
          Encryption
        </Text>
        <Group align="flex-end">
          <Select
            label="Encrypt backups with"
            data={[
              { value: 'Passphrase', label: 'A passphrase' },
              { value: 'Recipients', label: 'age public keys' },
              { value: 'None', label: 'Nothing (not recommended)' },
            ]}
            value={encKind}
            onChange={(v) => v && setEnc(v === 'None' ? 'None' : v === 'Recipients' ? { Recipients: { recipients: [] } } : { Passphrase: { passphrase_ref: 'backup-passphrase', work_factor: null } })}
            allowDeselect={false}
            w={240}
          />
          {encKind === 'Passphrase' && typeof enc === 'object' && 'Passphrase' in enc && (
            <Select label="Passphrase (a secret's name)" data={secrets} value={secrets.includes(enc.Passphrase.passphrase_ref) ? enc.Passphrase.passphrase_ref : null} onChange={(v) => v && setEnc({ Passphrase: { ...enc.Passphrase, passphrase_ref: v } })} placeholder="add it on the Secrets tab" w={260} />
          )}
        </Group>
        {encKind === 'Recipients' && typeof enc === 'object' && 'Recipients' in enc && (
          <Textarea
            label="Recipients (age1… public keys, one per line)"
            value={enc.Recipients.recipients.join('\n')}
            onChange={(e) => setEnc({ Recipients: { recipients: e.currentTarget.value.split(/\s+/).filter(Boolean) } })}
            autosize
            minRows={2}
            mt="xs"
          />
        )}
      </Card>
      <Card withBorder>
        <Text fw={600} mb="xs">
          Keep
        </Text>
        <Group>
          {(['keep_last', 'daily', 'weekly', 'monthly', 'yearly'] as const).map((k) => (
            <NumberInput key={k} label={k === 'keep_last' ? 'Most recent' : k[0]!.toUpperCase() + k.slice(1)} value={cfg.retention[k]} onChange={(v) => setCfg({ ...cfg, retention: { ...cfg.retention, [k]: Math.max(0, Number(v) || 0) } })} min={0} w={110} />
          ))}
          <NumberInput label="Compression (1–22)" value={cfg.compression_level} onChange={(v) => setCfg({ ...cfg, compression_level: Math.min(22, Math.max(1, Number(v) || 3)) })} min={1} max={22} w={140} />
        </Group>
      </Card>
      <ErrorBox error={save.error} />
      <Group justify="flex-end">
        <Button onClick={() => save.mutate({ ...settings, backups: cfg })} loading={save.isPending}>
          Save
        </Button>
      </Group>
    </Stack>
  );
}

function SecretsPanel({ secrets }: { secrets: Array<{ name: string; updated_at: string }> }) {
  const api = useApi();
  const invalidate = useInvalidate();
  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const [replacing, setReplacing] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const put = useMutation({
    mutationFn: ({ n, v }: { n: string; v: string }) => api.putSecret(n, v),
    onSuccess: (_r, { n }) => {
      invalidate(sk.server);
      setName('');
      setValue('');
      setReplacing(null);
      notifications.show({ message: `Secret "${n}" saved.`, color: 'teal' });
    },
  });
  const del = useMutation({
    mutationFn: (n: string) => api.deleteSecret(n),
    onSuccess: () => {
      invalidate(sk.server);
      setDeleting(null);
    },
  });
  return (
    <Stack gap="md" maw={640}>
      <Text size="sm" c="dimmed">
        Passphrases, API keys and tokens the settings refer to by name. Values are write-only: once saved they're never shown again, only replaced.
      </Text>
      <Table fz="sm" withTableBorder>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Name</Table.Th>
            <Table.Th>Updated</Table.Th>
            <Table.Th />
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {secrets.map((s) => (
            <Table.Tr key={s.name} data-testid="secret-row">
              <Table.Td>
                <Group gap={6}>
                  <IconKey size={14} />
                  <Code>{s.name}</Code>
                </Group>
              </Table.Td>
              <Table.Td>
                <When iso={s.updated_at} />
              </Table.Td>
              <Table.Td ta="right">
                <Group gap={2} justify="flex-end">
                  <Button size="compact-xs" variant="subtle" onClick={() => setReplacing(s.name)}>
                    Replace
                  </Button>
                  <Tooltip label="Delete">
                    <ActionIcon variant="subtle" color="red" aria-label={`Delete secret ${s.name}`} onClick={() => setDeleting(s.name)}>
                      <IconTrash size={16} />
                    </ActionIcon>
                  </Tooltip>
                </Group>
              </Table.Td>
            </Table.Tr>
          ))}
          {secrets.length === 0 && (
            <Table.Tr>
              <Table.Td colSpan={3}>
                <Text c="dimmed" size="sm">
                  No secrets yet.
                </Text>
              </Table.Td>
            </Table.Tr>
          )}
        </Table.Tbody>
      </Table>
      <Card withBorder>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            put.mutate({ n: name.trim(), v: value });
          }}
        >
          <Group align="flex-end">
            <TextInput label="Name" placeholder="backup-passphrase" value={name} onChange={(e) => setName(e.currentTarget.value)} pattern="[A-Za-z0-9._-]{1,128}" w={220} />
            <PasswordInput label="Value" value={value} onChange={(e) => setValue(e.currentTarget.value)} style={{ flex: 1 }} />
            <Button type="submit" loading={put.isPending} disabled={!name.trim() || !value}>
              Add secret
            </Button>
          </Group>
        </form>
        <ErrorBox error={put.error} />
      </Card>
      <Modal opened={!!replacing} onClose={() => setReplacing(null)} title={`Replace "${replacing}"`} centered size="sm">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            put.mutate({ n: replacing!, v: value });
          }}
        >
          <Stack gap="sm">
            <PasswordInput label="New value" value={value} onChange={(e) => setValue(e.currentTarget.value)} data-autofocus />
            <Group justify="flex-end">
              <Button variant="default" onClick={() => setReplacing(null)}>
                Cancel
              </Button>
              <Button type="submit" loading={put.isPending} disabled={!value}>
                Replace
              </Button>
            </Group>
          </Stack>
        </form>
      </Modal>
      <Confirm opened={!!deleting} title={`Delete secret "${deleting}"?`} confirmLabel="Delete" loading={del.isPending} onConfirm={() => deleting && del.mutate(deleting)} onClose={() => setDeleting(null)}>
        <Text size="sm">Refused while the settings still refer to it.</Text>
        <ErrorBox error={del.error} />
      </Confirm>
    </Stack>
  );
}
