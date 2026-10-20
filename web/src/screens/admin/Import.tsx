// QuickBooks import (Company → Import from QuickBooks; admin): how LibreBooks
// reaches the QBD company file (the bridge on the QuickBooks machine, the Web
// Connector, or the SDK in the desktop app), running a sync and watching it,
// the last sync's report, and the parity report (QBD's balances beside ours).
// Routes: `/import/qbd`, `…/sync`, `…/status`, `…/parity`, `…/qbwc.qwc`,
// `…/qbwc/password`.

import { Alert, Badge, Button, Card, Code, CopyButton, Group, Modal, NumberInput, PasswordInput, Progress, SegmentedControl, Select, SimpleGrid, Stack, Switch, Table, Tabs, Text, TextInput, Title, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconCheck, IconCopy, IconDownload, IconRefresh, IconScale } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { saveDownload } from '~/api/client';
import { useApi, useCompanyId } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import { useInvalidate, useQbdParity, useQbdSettings, useQbdStatus } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { ImportSettings, ParityReport, QbdTransport, SyncReport } from '~/api/types';
import { formatDateTime } from '~/lib/dates';
import { describeDuration, durationToMinutes, minutesToDuration } from '~/lib/format';
import { formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow } from '~/windows/WindowsProvider';
import classes from '../screens.module.css';
import { AdminGate, When } from './common';

type Tab = 'sync' | 'settings' | 'parity';

const TRANSPORT_LABEL: Record<QbdTransport, string> = { bridge: 'Bridge', qbwc: 'Web Connector', sdk: 'SDK (desktop app)' };

export function ImportWindow() {
  const win = useWindow();
  useEffect(() => win.setTitle('Import from QuickBooks'), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <AdminGate what="Importing from QuickBooks">
      <Import />
    </AdminGate>
  );
}

function Import() {
  const win = useWindow();
  const mobile = useIsMobile();
  const [tab, setTab] = useState<Tab>((win.desc.params.tab as Tab | undefined) ?? 'sync');
  const settings = useQbdSettings();
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="qbd-import">
      <div>
        <Title order={3}>Import from QuickBooks Desktop</Title>
        <Text size="sm" c="dimmed">
          Copies the company file into these books and keeps it current while you run both. QuickBooks stays the source until you stop syncing.
        </Text>
      </div>
      <Tabs value={tab} onChange={(v) => setTab((v as Tab) ?? 'sync')} keepMounted={false}>
        <Tabs.List>
          <Tabs.Tab value="sync">Sync</Tabs.Tab>
          <Tabs.Tab value="settings">Connection</Tabs.Tab>
          <Tabs.Tab value="parity">Parity</Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel value="sync" pt="md">
          <SyncPanel configured={!!settings.data} onConfigure={() => setTab('settings')} />
        </Tabs.Panel>
        <Tabs.Panel value="settings" pt="md">
          {settings.data ? <SettingsPanel settings={settings.data} /> : settings.error ? <Alert color="red">{describeError(settings.error)}</Alert> : <Text c="dimmed">Loading…</Text>}
        </Tabs.Panel>
        <Tabs.Panel value="parity" pt="md">
          <ParityPanel />
        </Tabs.Panel>
      </Tabs>
    </Stack>
  );
}

function SyncPanel({ configured, onConfigure }: { configured: boolean; onConfigure(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const invalidate = useInvalidate();
  const status = useQbdStatus();
  const sync = useMutation({
    mutationFn: (opts: { full?: boolean; sweep?: boolean }) => api.qbdSync(co, opts),
    onSuccess: () => invalidate(qk.qbdStatus(co)),
  });
  const st = status.data;
  const running = !!st?.running;
  const p = st?.progress;
  return (
    <Stack gap="md">
      <Card withBorder>
        <Group justify="space-between" align="flex-start" wrap="wrap">
          <div>
            <Group gap="xs">
              <Text fw={600}>{running ? 'Syncing…' : st?.last ? 'Idle' : 'Never synced'}</Text>
              {st?.source_id && (
                <Tooltip label="Which QuickBooks company these books are bound to">
                  <Code>{st.source_id}</Code>
                </Tooltip>
              )}
            </Group>
            {st?.last?.finished_at && !running && (
              <Text size="xs" c="dimmed">
                last sync <When iso={st.last.finished_at} />
                {st.last.aborted ? ` · stopped early: ${st.last.aborted}` : ''}
              </Text>
            )}
          </div>
          <Group gap="xs">
            <Button leftSection={<IconRefresh size={16} />} loading={running || sync.isPending} disabled={!configured} onClick={() => sync.mutate({})}>
              Sync now
            </Button>
            <Button variant="default" disabled={running || !configured} onClick={() => sync.mutate({ sweep: true })}>
              Sync and sweep deletions
            </Button>
            <Button variant="default" disabled={running || !configured} onClick={() => sync.mutate({ full: true })}>
              Full re-sync
            </Button>
          </Group>
        </Group>
        {running && p && (
          <Stack gap={4} mt="sm" data-testid="sync-progress">
            <Text size="sm">
              {p.phase} {p.entity}
              {p.total ? ` · ${p.done} of ${p.total}` : ''}
            </Text>
            <Progress value={p.total ? (100 * p.done) / p.total : 0} animated />
          </Stack>
        )}
        {!configured && (
          <Alert color="yellow" mt="sm">
            Nothing to sync from yet.{' '}
            <Button size="compact-xs" variant="light" onClick={onConfigure}>
              Set up the connection
            </Button>
          </Alert>
        )}
        {st?.last_error && <Alert color="red" mt="sm">{st.last_error}</Alert>}
        {sync.error && (
          <Alert color={isApiError(sync.error) && sync.error.code === 'running' ? 'yellow' : 'red'} mt="sm">
            {describeError(sync.error)}
          </Alert>
        )}
      </Card>
      {st?.last && <ReportView report={st.last} />}
    </Stack>
  );
}

function ReportView({ report }: { report: SyncReport }) {
  const mobile = useIsMobile();
  const totals = report.entities.reduce((s, e) => ({ created: s.created + e.created, updated: s.updated + e.updated, failed: s.failed + e.failed, skipped: s.skipped + e.skipped }), { created: 0, updated: 0, failed: 0, skipped: 0 });
  const unmapped = Object.entries(report.unmapped_fields);
  return (
    <Stack gap="md" data-testid="sync-report">
      <Card withBorder>
        <Group justify="space-between" mb="xs">
          <Text fw={600}>
            Last sync{report.full ? ' (full)' : ''}
            {report.partial_run ? ' (partial)' : ''}
          </Text>
          <Text size="xs" c="dimmed">
            {report.started_at ? formatDateTime(report.started_at) : ''}
            {report.requests !== null ? ` · ${report.requests} requests` : ''}
          </Text>
        </Group>
        <Group gap="lg" mb="sm">
          <Stat label="Created" value={totals.created} />
          <Stat label="Updated" value={totals.updated} />
          <Stat label="Skipped" value={totals.skipped} />
          <Stat label="Failed" value={totals.failed} color={totals.failed ? 'red' : undefined} />
        </Group>
        <Table fz="xs" withTableBorder>
          <Table.Thead>
            <Table.Tr>
              <Table.Th>Type</Table.Th>
              <Table.Th ta="right">Listed</Table.Th>
              <Table.Th ta="right">Created</Table.Th>
              <Table.Th ta="right">Updated</Table.Th>
              {!mobile && <Table.Th ta="right">Unchanged</Table.Th>}
              {!mobile && <Table.Th ta="right">Skipped</Table.Th>}
              <Table.Th ta="right">Failed</Table.Th>
              {!mobile && <Table.Th ta="right">Partial</Table.Th>}
              {!mobile && <Table.Th ta="right">Deleted</Table.Th>}
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {report.entities.map((e) => (
              <Table.Tr key={e.entity}>
                <Table.Td>{e.entity}</Table.Td>
                <Table.Td ta="right">{e.listed}</Table.Td>
                <Table.Td ta="right">{e.created}</Table.Td>
                <Table.Td ta="right">{e.updated}</Table.Td>
                {!mobile && <Table.Td ta="right">{e.unchanged}</Table.Td>}
                {!mobile && <Table.Td ta="right">{e.skipped}</Table.Td>}
                <Table.Td ta="right" c={e.failed ? 'red' : undefined}>
                  {e.failed}
                </Table.Td>
                {!mobile && <Table.Td ta="right">{e.partial}</Table.Td>}
                {!mobile && (
                  <Table.Td ta="right">
                    {e.deleted}
                    {e.deactivated ? ` (+${e.deactivated} inactive)` : ''}
                  </Table.Td>
                )}
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      </Card>
      {report.issues.length > 0 && (
        <Card withBorder>
          <Text fw={600} mb="xs">
            Issues
          </Text>
          <Stack gap={6}>
            {report.issues.map((i, n) => (
              <Group key={n} gap="xs" align="flex-start" wrap="nowrap">
                <Badge size="xs" color={i.severity === 'Error' ? 'red' : i.severity === 'Warning' ? 'yellow' : 'gray'} miw={60}>
                  {i.severity}
                </Badge>
                <div>
                  <Text size="sm">
                    <b>{i.record}</b> ({i.entity}
                    {i.qbd_id ? ` ${i.qbd_id}` : ''})
                  </Text>
                  <Text size="sm" c="dimmed">
                    {i.message}
                  </Text>
                </div>
              </Group>
            ))}
          </Stack>
        </Card>
      )}
      {(report.mismatches.length > 0 || report.gaps.length > 0 || unmapped.length > 0 || report.created_lists.length > 0) && (
        <SimpleGrid cols={{ base: 1, md: 2 }}>
          {report.mismatches.length > 0 && (
            <Card withBorder>
              <Text fw={600} mb="xs">
                Figures QBD stored differently
              </Text>
              <Table fz="xs">
                <Table.Tbody>
                  {report.mismatches.map((m, n) => (
                    <Table.Tr key={n}>
                      <Table.Td>
                        {m.record}
                        <Text span c="dimmed">
                          {' '}
                          · {m.kind}
                        </Text>
                      </Table.Td>
                      <Table.Td ta="right" className={classes.num}>
                        QBD {formatMoney(m.qbd)}
                      </Table.Td>
                      <Table.Td ta="right" className={classes.num}>
                        ours {formatMoney(m.ours)}
                      </Table.Td>
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            </Card>
          )}
          {report.gaps.length > 0 && (
            <Card withBorder>
              <Text fw={600} mb="xs">
                What the bridge can't give us
              </Text>
              {report.gaps.map((g) => (
                <Text key={g.qbd_type} size="sm">
                  <b>{g.qbd_type}</b> ({g.links} {g.links === 1 ? 'link' : 'links'}): {g.note}
                </Text>
              ))}
            </Card>
          )}
          {unmapped.length > 0 && (
            <Card withBorder>
              <Text fw={600} mb="xs">
                QBD fields with no home here
              </Text>
              {unmapped.map(([f, n]) => (
                <Text key={f} size="sm">
                  <Code>{f}</Code> on {n} {n === 1 ? 'record' : 'records'}
                </Text>
              ))}
            </Card>
          )}
          {report.created_lists.length > 0 && (
            <Card withBorder>
              <Text fw={600} mb="xs">
                List records the import had to create
              </Text>
              {report.created_lists.map((c) => (
                <Text key={c} size="sm">
                  {c}
                </Text>
              ))}
            </Card>
          )}
        </SimpleGrid>
      )}
    </Stack>
  );
}

function Stat({ label, value, color }: { label: string; value: number; color?: string }) {
  return (
    <div>
      <Text size="xs" c="dimmed">
        {label}
      </Text>
      <Text fw={600} c={color}>
        {value}
      </Text>
    </div>
  );
}

function SettingsPanel({ settings }: { settings: ImportSettings }) {
  const api = useApi();
  const co = useCompanyId();
  const invalidate = useInvalidate();
  const [s, setS] = useState<ImportSettings>(settings);
  const [key, setKey] = useState('');
  const [pw, setPw] = useState<string | null>(null);
  const [pwChoice, setPwChoice] = useState('');
  const [pwOpen, setPwOpen] = useState(false);
  useEffect(() => setS(settings), [settings]);
  const save = useMutation({
    mutationFn: () => api.putQbdSettings(co, { ...s, bridge: { ...s.bridge, bridge_key: key || undefined } }),
    onSuccess: () => {
      invalidate(qk.qbdAll(co));
      setKey('');
      notifications.show({ message: 'Connection settings saved.', color: 'teal' });
    },
  });
  const password = useMutation({
    mutationFn: (chosen: string | null) => api.qbwcPassword(co, chosen),
    onSuccess: (r) => {
      setPw(r.password);
      invalidate(qk.qbdSettings(co));
    },
  });
  const qwc = useMutation({ mutationFn: () => api.qbwcFile(co), onSuccess: saveDownload });
  const field = (name: string) => (isApiError(save.error) && (save.error.detail as { field?: string } | undefined)?.field === name ? save.error.message : undefined);
  return (
    <Stack gap="md" maw={720}>
      <SegmentedControl fullWidth data={(Object.keys(TRANSPORT_LABEL) as QbdTransport[]).map((t) => ({ value: t, label: TRANSPORT_LABEL[t] }))} value={s.transport} onChange={(v) => setS({ ...s, transport: v as QbdTransport })} />
      {s.transport === 'bridge' && (
        <Card withBorder>
          <Stack gap="sm">
            <Text size="sm" c="dimmed">
              The LibreBooks bridge runs on the Windows machine with QuickBooks and answers qbXML requests from this server.
            </Text>
            <TextInput label="Bridge URL" placeholder="http://192.168.1.40:8750" value={s.bridge.url} onChange={(e) => setS({ ...s, bridge: { ...s.bridge, url: e.currentTarget.value } })} error={field('bridge.url')} />
            <PasswordInput label="Bridge key" description={s.bridge.key_set ? 'A key is set. Paste a new one to replace it; it is never shown again.' : 'The shared key printed by the bridge when it starts.'} placeholder={s.bridge.key_set ? '•••••••• (set)' : ''} value={key} onChange={(e) => setKey(e.currentTarget.value)} error={field('bridge.bridge_key')} />
          </Stack>
        </Card>
      )}
      {s.transport === 'qbwc' && (
        <Card withBorder>
          <Stack gap="sm">
            <Text size="sm" c="dimmed">
              QuickBooks' own Web Connector polls this server. Download the .qwc file, add it in the Web Connector, and give it the password.
            </Text>
            <Group grow align="flex-start">
              <TextInput label="User name" value={s.qbwc.username} onChange={(e) => setS({ ...s, qbwc: { ...s.qbwc, username: e.currentTarget.value } })} error={field('qbwc.username')} />
              <Select
                label="File access"
                data={[
                  { value: 'ReadOnly', label: 'Read only' },
                  { value: 'ReadWrite', label: 'Read and write' },
                ]}
                value={s.qbwc.file_mode}
                onChange={(v) => v && setS({ ...s, qbwc: { ...s.qbwc, file_mode: v as 'ReadOnly' | 'ReadWrite' } })}
                allowDeselect={false}
              />
            </Group>
            <TextInput label="Company file (blank: whichever is open)" placeholder="C:\Users\Public\Documents\Intuit\QuickBooks\Company Files\Shop.QBW" value={s.qbwc.company_file ?? ''} onChange={(e) => setS({ ...s, qbwc: { ...s.qbwc, company_file: e.currentTarget.value || null } })} />
            <Group>
              <Button variant="default" size="xs" leftSection={<IconDownload size={14} />} loading={qwc.isPending} onClick={() => qwc.mutate()}>
                Download .qwc file
              </Button>
              <Button variant="default" size="xs" onClick={() => setPwOpen(true)}>
                {s.qbwc.password_set ? 'Change password' : 'Set password'}
              </Button>
              {s.qbwc.password_set && (
                <Badge variant="light" color="teal">
                  password set
                </Badge>
              )}
            </Group>
          </Stack>
        </Card>
      )}
      {s.transport === 'sdk' && (
        <Alert color="blue" variant="light">
          The desktop app talks to QuickBooks directly through the SDK when it runs on the same Windows machine. Nothing to configure here; open the company file in QuickBooks and sync.
        </Alert>
      )}
      <Card withBorder>
        <Group align="flex-end">
          <NumberInput label="Pause between requests (ms)" description="QuickBooks slows down when hammered" value={s.pause_ms} onChange={(v) => setS({ ...s, pause_ms: Number(v) || 0 })} min={0} max={60_000} step={50} w={220} error={field('pause_ms')} />
          <Switch label="Sync on a schedule" checked={s.auto_sync} onChange={(e) => setS({ ...s, auto_sync: e.currentTarget.checked })} />
          <NumberInput label="Every (minutes)" value={durationToMinutes(s.every) ?? 60} onChange={(v) => setS({ ...s, every: minutesToDuration(Number(v) || 60) })} min={5} step={5} w={160} disabled={!s.auto_sync} description={describeDuration(s.every)} />
        </Group>
      </Card>
      {save.error && !field('bridge.url') && !field('bridge.bridge_key') && !field('qbwc.username') && !field('pause_ms') && <Alert color="red">{describeError(save.error)}</Alert>}
      <Group justify="flex-end">
        <Button loading={save.isPending} onClick={() => save.mutate()}>
          Save
        </Button>
      </Group>
      <Modal
        opened={pwOpen}
        onClose={() => {
          setPwOpen(false);
          setPw(null);
          setPwChoice('');
        }}
        title="Web Connector password"
        centered
        size="sm"
      >
        {pw ? (
          <Stack gap="sm" data-testid="qbwc-password">
            <Text size="sm">Type this into the Web Connector's password box. It's shown once.</Text>
            <Group gap="xs" wrap="nowrap">
              <Code style={{ flex: 1, fontSize: 16 }}>{pw}</Code>
              <CopyButton value={pw}>
                {({ copied, copy }) => (
                  <Button size="xs" variant="default" leftSection={copied ? <IconCheck size={14} /> : <IconCopy size={14} />} onClick={copy}>
                    {copied ? 'Copied' : 'Copy'}
                  </Button>
                )}
              </CopyButton>
            </Group>
            <Group justify="flex-end">
              <Button
                onClick={() => {
                  setPwOpen(false);
                  setPw(null);
                }}
              >
                Done
              </Button>
            </Group>
          </Stack>
        ) : (
          <Stack gap="sm">
            <PasswordInput label="Choose a password (or leave blank to generate one)" value={pwChoice} onChange={(e) => setPwChoice(e.currentTarget.value)} data-autofocus />
            {password.error && <Alert color="red">{describeError(password.error)}</Alert>}
            <Group justify="flex-end">
              <Button variant="default" onClick={() => setPwOpen(false)}>
                Cancel
              </Button>
              <Button loading={password.isPending} onClick={() => password.mutate(pwChoice || null)}>
                {pwChoice ? 'Set' : 'Generate'}
              </Button>
            </Group>
          </Stack>
        )}
      </Modal>
    </Stack>
  );
}

function ParityPanel() {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const invalidate = useInvalidate();
  const parity = useQbdParity();
  const [onlyOff, setOnlyOff] = useState(true);
  const run = useMutation({
    mutationFn: () => api.qbdParityRun(co),
    onSuccess: () => {
      // 202: the report lands when QBD has answered; ask again shortly.
      setTimeout(() => invalidate(qk.qbdParity(co)), 600);
    },
  });
  const r: ParityReport | null | undefined = parity.data;
  const rows = r ? r.accounts.filter((a) => !onlyOff || a.difference !== 0) : [];
  return (
    <Stack gap="md">
      <Card withBorder>
        <Group justify="space-between" align="flex-start" wrap="wrap">
          <div>
            <Text fw={600}>Parity with QuickBooks</Text>
            <Text size="xs" c="dimmed">
              {r ? (
                <>
                  as of <When iso={r.generated_at} /> · {r.summary.accounts_matching} of {r.summary.accounts_compared} accounts match
                  {r.summary.accounts_off ? ` · ${r.summary.accounts_off} off by ${formatMoney(r.summary.total_abs_difference)} in total` : ''}
                </>
              ) : (
                'Asks QuickBooks for every account balance and sets it beside ours.'
              )}
            </Text>
          </div>
          <Group gap="xs">
            {r && <Switch label="Only differences" size="sm" checked={onlyOff} onChange={(e) => setOnlyOff(e.currentTarget.checked)} />}
            <Button leftSection={<IconScale size={16} />} loading={run.isPending} onClick={() => run.mutate()}>
              {r ? 'Run again' : 'Compare now'}
            </Button>
          </Group>
        </Group>
        {run.error && (
          <Alert color="red" mt="sm">
            {describeError(run.error)}
          </Alert>
        )}
        {r?.summary.notes.map((n) => (
          <Text key={n} size="xs" c="dimmed" mt="xs">
            {n}
          </Text>
        ))}
      </Card>
      {r && (
        <Card withBorder p={0}>
          <Table fz="sm" data-testid="parity-table">
            <Table.Thead>
              <Table.Tr>
                <Table.Th>Account</Table.Th>
                <Table.Th ta="right">QuickBooks</Table.Th>
                <Table.Th ta="right">LibreBooks</Table.Th>
                <Table.Th ta="right">Difference</Table.Th>
                {!mobile && <Table.Th>Note</Table.Th>}
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {rows.map((a) => (
                <Table.Tr key={a.qbd_list_id ?? a.full_name} data-off={a.difference !== 0 || undefined}>
                  <Table.Td>{a.full_name}</Table.Td>
                  <Table.Td ta="right" className={classes.num}>
                    {formatMoney(a.qbd)}
                  </Table.Td>
                  <Table.Td ta="right" className={classes.num}>
                    {formatMoney(a.ours)}
                  </Table.Td>
                  <Table.Td ta="right" className={classes.num} c={a.difference ? 'red' : 'dimmed'}>
                    {a.difference ? formatMoney(a.difference) : '—'}
                  </Table.Td>
                  {!mobile && (
                    <Table.Td>
                      <Text size="xs" c="dimmed">
                        {a.note ?? ''}
                      </Text>
                    </Table.Td>
                  )}
                </Table.Tr>
              ))}
              {rows.length === 0 && (
                <Table.Tr>
                  <Table.Td colSpan={5}>
                    <Text c="dimmed" size="sm" p="sm">
                      Every account matches.
                    </Text>
                  </Table.Td>
                </Table.Tr>
              )}
            </Table.Tbody>
          </Table>
          {r.trial_balance && (
            <Group p="sm" gap="lg">
              <Text size="xs" c="dimmed">
                Trial balance as of {r.trial_balance.as_of} ({r.trial_balance.basis}): QBD {formatMoney(r.trial_balance.qbd_debits)} / {formatMoney(r.trial_balance.qbd_credits)} · ours {formatMoney(r.trial_balance.ours_debits)} / {formatMoney(r.trial_balance.ours_credits)}
              </Text>
            </Group>
          )}
        </Card>
      )}
    </Stack>
  );
}
