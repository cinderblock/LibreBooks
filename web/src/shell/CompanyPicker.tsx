// `/`: who am I, which companies can I open. One company goes straight in
// (unless `/companies` asked for the list); no session goes to sign in. Server
// admins can start a company here: a new one from its information
// (`POST /companies`), or a `.lbk` company file (`POST /companies/import`);
// either way it is a NEW company, nothing existing is replaced, and the picker
// opens it.

import { Alert, Anchor, Button, Card, Center, FileInput, Group, Loader, Modal, Stack, Text, TextInput, Title } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconFolderOpen, IconPlus } from '@tabler/icons-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Navigate, useNavigate } from 'react-router';
import { blankCompanyInfo } from '~/api/blank';
import { useApi } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import { useSession } from '~/api/hooks';
import { sk } from '~/api/keys';
import type { CompanyId, CompanyInfo } from '~/api/types';
import { formatBytes } from '~/lib/format';
import { CompanyInfoFields } from '~/screens/CompanyInfoFields';

export function CompanyPicker({ force = false }: { /** Always show the list (the user menu's "Open another company"), even with one company. */ force?: boolean }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const session = useSession();
  const [creating, setCreating] = useState(false);
  const [importing, setImporting] = useState(false);
  if (session.isLoading) {
    return (
      <Center h="100dvh">
        <Loader />
      </Center>
    );
  }
  if (session.error) {
    if (isApiError(session.error) && session.error.code === 'unauthenticated') return <Navigate to="/signin" replace />;
    return (
      <Center h="100dvh" p="md">
        <Stack maw={460}>
          <Title order={2}>LibreBooks</Title>
          <Alert color="red" title="Can't reach the server">
            {describeError(session.error)}
          </Alert>
          <Text size="sm" c="dimmed">
            To try the interface without a server, open it in{' '}
            <Anchor href="/?fixtures=1">fixture mode</Anchor> (synthetic demo books held in memory).
          </Text>
        </Stack>
      </Center>
    );
  }
  const companies = session.data?.companies ?? [];
  const admin = session.data?.user.role === 'admin';
  if (!force && companies.length === 1) return <Navigate to={`/c/${encodeURIComponent(companies[0]!.id)}/home`} replace />;
  /** A company made just now: the session lists it, then it opens. */
  const openNew = (id: CompanyId) => {
    void qc.invalidateQueries({ queryKey: sk.session });
    navigate(`/c/${encodeURIComponent(id)}/home`);
  };
  return (
    <Center h="100dvh" p="md">
      <Stack w={460} maw="100%" data-testid="company-picker">
        <Title order={2}>Open a company</Title>
        {companies.length === 0 && (
          <Text c="dimmed">
            You don't have access to any company yet.
            {admin ? ' Create one below, or open a company file.' : ''}
          </Text>
        )}
        {companies.map((c) => (
          <Card key={c.id} withBorder>
            <Button variant="subtle" justify="space-between" fullWidth onClick={() => navigate(`/c/${encodeURIComponent(c.id)}/home`)}>
              {c.name}
            </Button>
          </Card>
        ))}
        {admin && (
          <Group gap="xs" mt="sm">
            <Button variant="default" leftSection={<IconPlus size={16} />} onClick={() => setCreating(true)}>
              New company…
            </Button>
            <Button variant="default" leftSection={<IconFolderOpen size={16} />} onClick={() => setImporting(true)}>
              Open a .lbk file…
            </Button>
          </Group>
        )}
        <NewCompanyModal opened={creating} onClose={() => setCreating(false)} onCreated={openNew} />
        <ImportCompanyModal opened={importing} onClose={() => setImporting(false)} onCreated={openNew} />
      </Stack>
    </Center>
  );
}

/** QBD's New Company: the information, then the books are empty and ready. */
function NewCompanyModal({ opened, onClose, onCreated }: { opened: boolean; onClose(): void; onCreated(id: CompanyId): void }) {
  const api = useApi();
  const [draft, setDraft] = useState<CompanyInfo>(() => blankCompanyInfo(''));
  useEffect(() => {
    if (opened) setDraft(blankCompanyInfo(''));
  }, [opened]);
  const create = useMutation({
    mutationFn: () => api.createCompany({ ...draft, name: draft.name.trim() }),
    onSuccess: ({ id }) => {
      notifications.show({ message: `"${draft.name.trim()}" is ready. Its chart of accounts is empty; add accounts as you go, or import from QuickBooks.`, color: 'teal', autoClose: 8_000 });
      onClose();
      onCreated(id);
    },
  });
  const field = (name: string) => (isApiError(create.error) && (create.error.detail as { field?: string } | undefined)?.field === name ? create.error.message : undefined);
  const named = !!draft.name.trim();
  return (
    <Modal opened={opened} onClose={onClose} title="New company" size="lg" centered>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (named) create.mutate();
        }}
      >
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            A new, empty company beside the ones you have. Everything here can be changed later under Company → Company Information.
          </Text>
          <CompanyInfoFields draft={draft} onChange={setDraft} fieldError={field} />
          {create.error && !field('name') && !field('fiscal_year_start_month') && <Alert color="red">{describeError(create.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" loading={create.isPending} disabled={!named}>
              Create company
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}

/** A `.lbk` (from Backups → Export, or the desktop app) becomes a new company. */
function ImportCompanyModal({ opened, onClose, onCreated }: { opened: boolean; onClose(): void; onCreated(id: CompanyId): void }) {
  const api = useApi();
  const [file, setFile] = useState<File | null>(null);
  const [name, setName] = useState('');
  useEffect(() => {
    if (opened) {
      setFile(null);
      setName('');
    }
  }, [opened]);
  const run = useMutation({
    mutationFn: () => api.importCompany(file!, name.trim() || null),
    onSuccess: (r) => {
      notifications.show({ message: `"${r.name}" is a new company, from a file made with LibreBooks ${r.from.app_version} on ${r.from.created_at.slice(0, 10)}.`, color: 'teal', autoClose: 8_000 });
      onClose();
      onCreated(r.id);
    },
  });
  const suggested = file ? file.name.replace(/\.lbk$/i, '') : '';
  return (
    <Modal opened={opened} onClose={onClose} title="Open a company file" centered>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (file) run.mutate();
        }}
      >
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            A <b>.lbk</b> company file (Backups → Export company file, or the desktop app) becomes a <b>new</b> company here. Nothing you already have is replaced.
          </Text>
          <FileInput label="Company file" placeholder="Choose a .lbk file" accept=".lbk,application/zip" value={file} onChange={setFile} clearable description={file ? `${formatBytes(file.size)}; large files take a while to upload` : undefined} aria-label="Company file" />
          <TextInput label="Name for the company" placeholder={suggested || 'The name in the file'} value={name} onChange={(e) => setName(e.currentTarget.value)} aria-label="Name for the company" />
          {run.error && (
            <Alert color="red">
              {isApiError(run.error) && run.error.code === 'damaged' ? `That file isn't a sound LibreBooks company file: ${run.error.message}` : describeError(run.error)}
            </Alert>
          )}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" loading={run.isPending} disabled={!file}>
              Open as a new company
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}
