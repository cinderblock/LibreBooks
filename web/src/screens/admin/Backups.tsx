// Backups (Company → Backups; admin on the company): the scheduler's state,
// every backup of this company in every destination, Back Up Now, Verify,
// Restore (into a NEW company — nothing existing is replaced), and Export
// (a portable company file).

import { Alert, Button, Card, Group, Modal, PasswordInput, Stack, Table, Text, Textarea, TextInput, Title } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconDatabaseExport, IconDeviceFloppy, IconRefresh } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { saveDownload } from '~/api/client';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useBackups, useBackupsStatus, useInvalidate } from '~/api/hooks';
import { qk, sk } from '~/api/keys';
import type { BackupManifest, RestoreRequest, VerifyReport } from '~/api/types';
import { formatDateTime } from '~/lib/dates';
import { formatBytes } from '~/lib/format';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow } from '~/windows/WindowsProvider';
import classes from '../screens.module.css';
import { AdminGate, When } from './common';

type Target = { destination: number; destination_name: string; manifest: BackupManifest };

export function BackupsWindow() {
  const win = useWindow();
  useEffect(() => win.setTitle('Backups'), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <AdminGate what="Backups">
      <Backups />
    </AdminGate>
  );
}

function Backups() {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const list = useBackups();
  const status = useBackupsStatus();
  const invalidate = useInvalidate();
  const [verifying, setVerifying] = useState<Target | null>(null);
  const [restoring, setRestoring] = useState<Target | null>(null);
  const run = useMutation({
    mutationFn: () => api.runBackup(co),
    onSuccess: (r) => {
      invalidate(qk.backupsAll(co), sk.server);
      const failed = r.destinations.filter((d) => !d.stored);
      notifications.show({ message: failed.length ? `Backed up, but ${failed.length} destination(s) failed: ${failed.map((d) => d.error).join('; ')}` : `Backed up (${formatBytes(r.manifest.stored_size)} stored).`, color: failed.length ? 'yellow' : 'teal' });
    },
  });
  const exportCo = useMutation({
    mutationFn: () => api.exportCompany(co),
    onSuccess: (file) => saveDownload(file),
  });
  const st = status.data;
  const rows = list.data?.backups ?? [];
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="backups">
      <Group justify="space-between" align="flex-start">
        <div>
          <Title order={3}>Backups</Title>
          {st && (
            <Text size="sm" c="dimmed">
              {st.enabled ? `Scheduled · to ${st.destinations.join(', ') || 'nowhere yet'}` : 'No schedule (set one under Server Settings → Backups)'}
              {st.scheduler?.next_run ? ` · next ${formatDateTime(st.scheduler.next_run)}` : ''}
            </Text>
          )}
        </div>
        <Group gap="xs">
          <Button variant="default" leftSection={<IconDatabaseExport size={16} />} loading={exportCo.isPending} onClick={() => exportCo.mutate()}>
            Export company file
          </Button>
          <Button leftSection={<IconDeviceFloppy size={16} />} loading={run.isPending} onClick={() => run.mutate()}>
            Back up now
          </Button>
        </Group>
      </Group>
      {st?.scheduler?.last?.error && <Alert color="red">The last scheduled backup failed: {st.scheduler.last.error}</Alert>}
      {(run.error || exportCo.error) && <Alert color="red">{describeError(run.error ?? exportCo.error)}</Alert>}
      {(list.data?.errors ?? []).map((e) => (
        <Alert key={e} color="yellow">
          {e}
        </Alert>
      ))}
      <Card withBorder p={0}>
        <Table fz="sm" highlightOnHover>
          <Table.Thead>
            <Table.Tr>
              <Table.Th>Taken</Table.Th>
              {!mobile && <Table.Th>Where</Table.Th>}
              <Table.Th ta="right">Size</Table.Th>
              {!mobile && <Table.Th>Version</Table.Th>}
              <Table.Th />
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {rows.map((b) => (
              <Table.Tr key={`${b.destination}-${b.manifest.id}`} data-testid="backup-row">
                <Table.Td>
                  <Text size="sm">{formatDateTime(b.manifest.created_at)}</Text>
                  <Text size="xs" c="dimmed">
                    <When iso={b.manifest.created_at} />
                    {b.manifest.encryption !== 'none' ? ' · encrypted' : ''}
                  </Text>
                </Table.Td>
                {!mobile && <Table.Td>{b.destination_name}</Table.Td>}
                <Table.Td ta="right">
                  <Text size="sm">{formatBytes(b.manifest.stored_size)}</Text>
                  {!mobile && (
                    <Text size="xs" c="dimmed">
                      {formatBytes(b.manifest.database_size)} raw
                    </Text>
                  )}
                </Table.Td>
                {!mobile && (
                  <Table.Td>
                    <Text size="xs">
                      {b.manifest.app_version} · schema {b.manifest.schema_version}
                    </Text>
                  </Table.Td>
                )}
                <Table.Td ta="right">
                  <Group gap={4} justify="flex-end" wrap="nowrap">
                    <Button size="compact-xs" variant="subtle" onClick={() => setVerifying(b)}>
                      Verify
                    </Button>
                    <Button size="compact-xs" variant="light" onClick={() => setRestoring(b)}>
                      Restore…
                    </Button>
                  </Group>
                </Table.Td>
              </Table.Tr>
            ))}
            {rows.length === 0 && !list.isLoading && (
              <Table.Tr>
                <Table.Td colSpan={5}>
                  <Text c="dimmed" size="sm" p="sm">
                    No backups yet. "Back up now" makes one in every destination.
                  </Text>
                </Table.Td>
              </Table.Tr>
            )}
          </Table.Tbody>
        </Table>
      </Card>
      <VerifyModal target={verifying} onClose={() => setVerifying(null)} />
      <RestoreModal target={restoring} onClose={() => setRestoring(null)} />
    </Stack>
  );
}

/** Passphrase or age identities, for encrypted backups. */
function UnlockFields({ value, onChange }: { value: RestoreRequest; onChange(v: RestoreRequest): void }) {
  const [mode, setMode] = useState<'passphrase' | 'identities'>('passphrase');
  return (
    <Stack gap="xs">
      <Group gap="xs">
        <Button size="compact-xs" variant={mode === 'passphrase' ? 'light' : 'subtle'} onClick={() => setMode('passphrase')}>
          Passphrase
        </Button>
        <Button size="compact-xs" variant={mode === 'identities' ? 'light' : 'subtle'} onClick={() => setMode('identities')}>
          age identity
        </Button>
      </Group>
      {mode === 'passphrase' ? (
        <PasswordInput label="Passphrase" value={value.passphrase ?? ''} onChange={(e) => onChange({ ...value, passphrase: e.currentTarget.value || undefined, identities: undefined })} />
      ) : (
        <Textarea label="Identities (AGE-SECRET-KEY-…, one per line)" value={(value.identities ?? []).join('\n')} onChange={(e) => onChange({ ...value, identities: e.currentTarget.value.split(/\s+/).filter(Boolean), passphrase: undefined })} autosize minRows={2} />
      )}
    </Stack>
  );
}

function VerifyModal({ target, onClose }: { target: Target | null; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const [unlock, setUnlock] = useState<RestoreRequest>({});
  const [report, setReport] = useState<VerifyReport | null>(null);
  useEffect(() => {
    setReport(null);
    setUnlock({});
  }, [target]);
  const verify = useMutation({
    mutationFn: () => api.verifyBackup(co, target!.manifest.id, target!.destination, unlock),
    onSuccess: setReport,
  });
  return (
    <Modal opened={!!target} onClose={onClose} title="Verify backup" centered>
      {target && (
        <Stack gap="sm">
          <Text size="sm">
            {formatDateTime(target.manifest.created_at)} in {target.destination_name}. The checksum is always checked; with the key, the contents are opened and checked too.
          </Text>
          {target.manifest.encryption !== 'none' && <UnlockFields value={unlock} onChange={setUnlock} />}
          {report && (
            <Alert color={report.problems.length ? (report.checksum_ok ? 'yellow' : 'red') : 'teal'} title={report.checksum_ok ? (report.contents_checked ? 'Checksum and contents OK' : 'Checksum OK') : 'Checksum FAILED'} data-testid="verify-report">
              {report.problems.map((p) => (
                <Text key={p} size="sm">
                  {p}
                </Text>
              ))}
            </Alert>
          )}
          {verify.error && <Alert color="red">{describeError(verify.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Close
            </Button>
            <Button leftSection={<IconRefresh size={14} />} loading={verify.isPending} onClick={() => verify.mutate()}>
              Verify
            </Button>
          </Group>
        </Stack>
      )}
    </Modal>
  );
}

function RestoreModal({ target, onClose }: { target: Target | null; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const invalidate = useInvalidate();
  const [name, setName] = useState('');
  const [unlock, setUnlock] = useState<RestoreRequest>({});
  useEffect(() => {
    if (target) setName(`${target.manifest.company_name} (restored from ${target.manifest.created_at.slice(0, 10)})`);
    setUnlock({});
  }, [target]);
  const restore = useMutation({
    mutationFn: () => api.restoreBackup(co, target!.manifest.id, target!.destination, { ...unlock, name: name.trim() || undefined }),
    onSuccess: (r) => {
      invalidate(sk.session);
      notifications.show({ title: 'Restored', message: `"${r.name}" is a new company in the company list; this one is untouched.`, color: 'teal', autoClose: 10_000 });
      onClose();
    },
  });
  return (
    <Modal opened={!!target} onClose={onClose} title="Restore as a new company" centered>
      {target && (
        <Stack gap="sm">
          <Alert color="blue" variant="light">
            Restoring never replaces anything. The backup becomes a <b>new</b> company you can open beside this one, compare, and keep or delete.
          </Alert>
          <TextInput label="Name for the restored company" value={name} onChange={(e) => setName(e.currentTarget.value)} data-autofocus />
          {target.manifest.encryption !== 'none' && <UnlockFields value={unlock} onChange={setUnlock} />}
          {restore.error && <Alert color="red">{describeError(restore.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button loading={restore.isPending} onClick={() => restore.mutate()}>
              Restore
            </Button>
          </Group>
        </Stack>
      )}
    </Modal>
  );
}
