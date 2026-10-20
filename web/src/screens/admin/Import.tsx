// QuickBooks import (Company → Import from QuickBooks; admin): how LibreBooks
// reaches the QBD company file (the bridge on the QuickBooks machine, the Web
// Connector, or the SDK in the desktop app), running a sync and watching it,
// the last sync's report, and the parity report (QBD's balances beside ours).
// One `ImportStatus` document carries all of it (`GET /import/qbd`); the
// settings are lb-qbd's generated `ImportSettings`. Routes: `/import/qbd`,
// `…/sync`, `…/status`, `…/parity`, `…/qbwc.qwc`, `…/qbwc/password`.

import { Alert, Badge, Button, Card, Code, CopyButton, Group, List, Modal, NumberInput, PasswordInput, Progress, SegmentedControl, Select, SimpleGrid, Stack, Switch, Table, Tabs, Text, TextInput, Title, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconCheck, IconCopy, IconDownload, IconRefresh, IconScale } from '@tabler/icons-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { saveDownload } from '~/api/client';
import { useApi, useCompanyId } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import { useQbdParity, useQbdStatus } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { FileMode, ImportSettings, ImportStatus, QbdTransport, RunningJob, SyncReport } from '~/api/types';
import { formatDateTime } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow } from '~/windows/WindowsProvider';
import classes from '../screens.module.css';
import { AdminGate, When } from './common';

type Tab = 'sync' | 'settings' | 'parity';

const TRANSPORT_LABEL: Record<QbdTransport, string> = { bridge: 'Bridge', qbwc: 'Web Connector', sdk: 'SDK (desktop app)' };
const FILE_MODE_LABEL: Record<FileMode, string> = { dont_care: 'However QuickBooks has it open', single_user: 'Single-user', multi_user: 'Multi-user' };

/**
 * What stops a job before the server is asked, mirroring lb-qbd's
 * `ImportSettings::problems()` plus what only the status knows (the key and
 * SDK availability). The server is the judge; this just greys the buttons.
 */
function setupProblems(st: ImportStatus): string[] {
  const s = st.settings;
  const out: string[] = [];
  if (s.transport === 'bridge') {
    if (!s.bridge.url.trim()) out.push('the bridge URL is empty');
    if (!st.bridge_key_set) out.push("the bridge's API key hasn't been set");
  } else if (s.transport === 'qbwc') {
    if (!s.qbwc.user_name.trim()) out.push('the Web Connector user name is empty');
    if (!st.qbwc_password_set) out.push('the Web Connector password has not been set');
  } else if (!st.sdk_available) {
    out.push('the QuickBooks SDK needs LibreBooks running on the Windows machine that has QuickBooks');
  }
  return out;
}

/** A 400 `invalid` from `PUT /import/qbd` or a job start lists what to fix in `detail.problems`. */
function problemsOf(e: unknown): string[] {
  if (!isApiError(e)) return [];
  const d = e.detail as { problems?: unknown } | undefined;
  return Array.isArray(d?.problems) ? d.problems.filter((p): p is string => typeof p === 'string') : [];
}

function ProblemAlert({ error, color = 'red' }: { error: unknown; color?: string }) {
  const problems = problemsOf(error);
  return (
    <Alert color={color} mt="sm">
      {problems.length ? (
        <>
          <Text size="sm">Fix these first:</Text>
          <List size="sm">
            {problems.map((p) => (
              <List.Item key={p}>{p}</List.Item>
            ))}
          </List>
        </>
      ) : (
        describeError(error)
      )}
    </Alert>
  );
}

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
  const status = useQbdStatus();
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="qbd-import">
      <div>
        <Title order={3}>Import from QuickBooks Desktop</Title>
        <Text size="sm" c="dimmed">
          Copies the company file into these books and keeps it current while you run both. QuickBooks stays the source until you stop syncing.
        </Text>
      </div>
      {status.error && <Alert color="red">{describeError(status.error)}</Alert>}
      <Tabs value={tab} onChange={(v) => setTab((v as Tab) ?? 'sync')} keepMounted={false}>
        <Tabs.List>
          <Tabs.Tab value="sync">Sync</Tabs.Tab>
          <Tabs.Tab value="settings">Connection</Tabs.Tab>
          <Tabs.Tab value="parity">Parity</Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel value="sync" pt="md">
          {status.data ? <SyncPanel status={status.data} onConfigure={() => setTab('settings')} /> : <Text c="dimmed">Loading…</Text>}
        </Tabs.Panel>
        <Tabs.Panel value="settings" pt="md">
          {status.data ? <SettingsPanel status={status.data} /> : <Text c="dimmed">Loading…</Text>}
        </Tabs.Panel>
        <Tabs.Panel value="parity" pt="md">
          <ParityPanel status={status.data} />
        </Tabs.Panel>
      </Tabs>
    </Stack>
  );
}

/** Put the status a write returned into the cache so the window reflects it before the next poll. */
function useApplyStatus() {
  const qc = useQueryClient();
  const co = useCompanyId();
  return (st: ImportStatus) => {
    qc.setQueryData<ImportStatus>(qk.qbdStatus(co), st);
    void qc.invalidateQueries({ queryKey: qk.qbdAll(co) });
  };
}

function RunningLine({ job }: { job: RunningJob }) {
  const p = job.progress;
  if (job.waiting_for_web_connector) {
    return (
      <Alert color="blue" variant="light" mt="sm" data-testid="sync-progress">
        Waiting for the QuickBooks Web Connector to poll (started <When iso={job.started_at} />). Open the Web Connector and click Update Selected, or wait for its schedule.
      </Alert>
    );
  }
  return (
    <Stack gap={4} mt="sm" data-testid="sync-progress">
      <Text size="sm">
        {p ? `${p.phase} ${p.entity}${p.total ? ` · ${p.done} of ${p.total}` : ''}` : job.kind === 'parity' ? 'Comparing balances…' : 'Starting…'}
        {job.web_connector_connected ? ' · Web Connector connected' : ''}
      </Text>
      <Progress value={p?.total ? (100 * p.done) / p.total : job.percent} animated />
    </Stack>
  );
}

function SyncPanel({ status: st, onConfigure }: { status: ImportStatus; onConfigure(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const apply = useApplyStatus();
  const sync = useMutation({ mutationFn: (opts: { full?: boolean; sweep?: boolean }) => api.qbdSync(co, opts), onSuccess: apply });
  const job = st.running;
  const running = !!job;
  const last = st.last_sync;
  const problems = setupProblems(st);
  const configured = problems.length === 0;
  return (
    <Stack gap="md">
      <Card withBorder>
        <Group justify="space-between" align="flex-start" wrap="wrap">
          <div>
            <Group gap="xs">
              <Text fw={600}>{job ? (job.kind === 'sync' ? 'Syncing…' : 'Comparing with QuickBooks…') : last ? 'Idle' : 'Never synced'}</Text>
              {last?.source_id && (
                <Tooltip label="Which QuickBooks company these books are bound to">
                  <Code>{last.source_id}</Code>
                </Tooltip>
              )}
              <Badge variant="light" color="gray">
                {TRANSPORT_LABEL[st.settings.transport]}
              </Badge>
            </Group>
            {last?.finished_at && !running && (
              <Text size="xs" c="dimmed">
                last sync <When iso={last.finished_at} />
                {last.aborted ? ` · stopped early: ${last.aborted}` : ''}
              </Text>
            )}
          </div>
          <Group gap="xs">
            <Button leftSection={<IconRefresh size={16} />} loading={(running && job.kind === 'sync') || sync.isPending} disabled={running || !configured} onClick={() => sync.mutate({})}>
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
        {job && <RunningLine job={job} />}
        {!configured && (
          <Alert color="yellow" mt="sm">
            Nothing to sync from yet: {problems.join('; ')}.{' '}
            <Button size="compact-xs" variant="light" onClick={onConfigure}>
              Set up the connection
            </Button>
          </Alert>
        )}
        {st.last_error && <Alert color="red" mt="sm">{st.last_error}</Alert>}
        {sync.error && <ProblemAlert error={sync.error} color={isApiError(sync.error) && sync.error.code === 'running' ? 'yellow' : 'red'} />}
      </Card>
      {last && <ReportView report={last} />}
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

function SettingsPanel({ status }: { status: ImportStatus }) {
  const api = useApi();
  const co = useCompanyId();
  const apply = useApplyStatus();
  const [s, setS] = useState<ImportSettings>(status.settings);
  const [key, setKey] = useState('');
  const [pw, setPw] = useState<string | null>(null);
  const [pwChoice, setPwChoice] = useState('');
  const [pwOpen, setPwOpen] = useState(false);
  useEffect(() => setS(status.settings), [status.settings]);
  const save = useMutation({
    mutationFn: () => api.putQbdSettings(co, s, key || null),
    onSuccess: (st) => {
      apply(st);
      setKey('');
      notifications.show({ message: 'Connection settings saved.', color: 'teal' });
    },
  });
  const password = useMutation({
    mutationFn: (chosen: string | null) => api.qbwcPassword(co, chosen),
    onSuccess: (generated) => {
      void apply({ ...status, qbwc_password_set: true });
      if (generated) setPw(generated);
      else {
        closePassword();
        notifications.show({ message: 'Web Connector password set.', color: 'teal' });
      }
    },
  });
  const qwc = useMutation({ mutationFn: () => api.qbwcFile(co), onSuccess: saveDownload });
  const closePassword = () => {
    setPwOpen(false);
    setPw(null);
    setPwChoice('');
  };
  const bridge = (patch: Partial<ImportSettings['bridge']>) => setS({ ...s, bridge: { ...s.bridge, ...patch } });
  const qbwc = (patch: Partial<ImportSettings['qbwc']>) => setS({ ...s, qbwc: { ...s.qbwc, ...patch } });
  const companyFile = <TextInput label="Company file (blank: whichever QuickBooks has open)" placeholder="C:\Users\Public\Documents\Intuit\QuickBooks\Company Files\Shop.QBW" value={s.company_file} onChange={(e) => setS({ ...s, company_file: e.currentTarget.value })} />;
  return (
    <Stack gap="md" maw={720}>
      <SegmentedControl fullWidth data={(Object.keys(TRANSPORT_LABEL) as QbdTransport[]).map((t) => ({ value: t, label: TRANSPORT_LABEL[t] }))} value={s.transport} onChange={(v) => setS({ ...s, transport: v as QbdTransport })} />
      {s.transport === 'bridge' && (
        <Card withBorder>
          <Stack gap="sm">
            <Text size="sm" c="dimmed">
              The LibreBooks bridge runs on the Windows machine with QuickBooks and answers qbXML requests from this server.
            </Text>
            <TextInput label="Bridge URL" placeholder="http://192.168.1.40:8750" value={s.bridge.url} onChange={(e) => bridge({ url: e.currentTarget.value })} />
            <PasswordInput
              label="Bridge key"
              description={status.bridge_key_set ? 'A key is set. Paste a new one to replace it; it is never shown again.' : 'The shared key printed by the bridge when it starts.'}
              placeholder={status.bridge_key_set ? '•••••••• (set)' : ''}
              value={key}
              onChange={(e) => setKey(e.currentTarget.value)}
              rightSection={status.bridge_key_set && !key ? <Badge size="xs" variant="light" color="teal">set</Badge> : undefined}
              rightSectionWidth={status.bridge_key_set && !key ? 48 : undefined}
            />
            <NumberInput label="Pause between requests (ms)" description="QuickBooks slows down when hammered" value={s.bridge.delay_ms} onChange={(v) => bridge({ delay_ms: Math.max(0, Math.round(Number(v) || 0)) })} min={0} max={60_000} step={50} w={240} />
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
              <TextInput label="User name" description="Unique across this server's companies" value={s.qbwc.user_name} onChange={(e) => qbwc({ user_name: e.currentTarget.value })} />
              <NumberInput label="Run every (minutes)" description="Blank: only when you click Update Selected" value={s.qbwc.run_every_minutes ?? ''} onChange={(v) => qbwc({ run_every_minutes: v === '' ? null : Math.max(0, Math.round(Number(v) || 0)) })} min={0} step={5} allowDecimal={false} />
            </Group>
            <TextInput label="App URL (optional)" description="Overrides the server's public URL in the .qwc; must be https:// or http://localhost" placeholder="https://books.example.com" value={s.qbwc.app_url ?? ''} onChange={(e) => qbwc({ app_url: e.currentTarget.value.trim() || null })} />
            {companyFile}
            <Group>
              <Button variant="default" size="xs" leftSection={<IconDownload size={14} />} loading={qwc.isPending} onClick={() => qwc.mutate()}>
                Download .qwc file
              </Button>
              <Button variant="default" size="xs" onClick={() => setPwOpen(true)}>
                {status.qbwc_password_set ? 'Change password' : 'Set password'}
              </Button>
              {status.qbwc_password_set && (
                <Badge variant="light" color="teal">
                  password set
                </Badge>
              )}
            </Group>
            {qwc.error && <Alert color="red">{describeError(qwc.error)}</Alert>}
          </Stack>
        </Card>
      )}
      {s.transport === 'sdk' && (
        <Card withBorder>
          <Stack gap="sm">
            <Alert color={status.sdk_available ? 'blue' : 'yellow'} variant="light">
              {status.sdk_available
                ? 'The desktop app talks to QuickBooks directly through the SDK on this Windows machine. Open the company file in QuickBooks and sync.'
                : 'The QuickBooks SDK needs LibreBooks running on the Windows machine that has QuickBooks; this server has no SDK support. Use the bridge or the Web Connector instead.'}
            </Alert>
            <Select label="Open the company file" data={(Object.keys(FILE_MODE_LABEL) as FileMode[]).map((m) => ({ value: m, label: FILE_MODE_LABEL[m] }))} value={s.sdk.file_mode} onChange={(v) => v && setS({ ...s, sdk: { file_mode: v as FileMode } })} allowDeselect={false} />
            {companyFile}
          </Stack>
        </Card>
      )}
      {save.error && <ProblemAlert error={save.error} />}
      <Group justify="flex-end">
        <Button loading={save.isPending} onClick={() => save.mutate()}>
          Save
        </Button>
      </Group>
      <Modal opened={pwOpen} onClose={closePassword} title="Web Connector password" centered size="sm">
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
              <Button onClick={closePassword}>Done</Button>
            </Group>
          </Stack>
        ) : (
          <Stack gap="sm">
            <PasswordInput label="Choose a password (or leave blank to generate one)" description="At least 8 characters" value={pwChoice} onChange={(e) => setPwChoice(e.currentTarget.value)} data-autofocus />
            {password.error && <Alert color="red">{describeError(password.error)}</Alert>}
            <Group justify="flex-end">
              <Button variant="default" onClick={closePassword}>
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

function ParityPanel({ status }: { status: ImportStatus | undefined }) {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const apply = useApplyStatus();
  const parity = useQbdParity();
  const [onlyOff, setOnlyOff] = useState(true);
  const run = useMutation({ mutationFn: () => api.qbdParityRun(co), onSuccess: apply });
  const r = parity.data?.report ?? null;
  const running = !!parity.data?.running;
  const busy = running || !!status?.running;
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
            <Button leftSection={<IconScale size={16} />} loading={running || run.isPending} disabled={busy} onClick={() => run.mutate()}>
              {r ? 'Run again' : 'Compare now'}
            </Button>
          </Group>
        </Group>
        {parity.data?.job && parity.data.running && <RunningLine job={parity.data.job} />}
        {parity.error && <Alert color="red" mt="sm">{describeError(parity.error)}</Alert>}
        {parity.data?.error && !running && <Alert color="red" mt="sm">{parity.data.error}</Alert>}
        {run.error && <ProblemAlert error={run.error} color={isApiError(run.error) && run.error.code === 'running' ? 'yellow' : 'red'} />}
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
