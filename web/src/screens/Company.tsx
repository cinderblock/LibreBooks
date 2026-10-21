// Company: the company information (editable), QBD's Set Closing Date, the
// administrative windows (users, server, backups, import, merge…), and this
// device's preferences: the keymap (desktop app vs browser) and what Enter does.

import { Alert, Button, Card, Group, Modal, SegmentedControl, SimpleGrid, Skeleton, Stack, Table, Text, Title, UnstyledButton } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconArrowsJoin, IconCloudUpload, IconDatabaseExport, IconFileImport, IconLink, IconLock, IconPaperclip, IconPencil, IconServer, IconUsers } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import { useClosingDate, useCompanyInfo, useInvalidate, useIsAdmin } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { CompanyInfo } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import type { EnterBehavior } from '~/inputs/LineGrid';
import { useKeyboard } from '~/keyboard/KeyboardProvider';
import { COMMANDS } from '~/keyboard/commands';
import { formatCombo } from '~/keyboard/combo';
import { differencesFromQbd } from '~/keyboard/keymaps';
import { formatDate } from '~/lib/dates';
import { useEnterBehavior } from '~/lib/prefs';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { CompanyInfoFields, MONTHS } from './CompanyInfoFields';
import classes from './screens.module.css';

export function Company() {
  const { data: info, isLoading } = useCompanyInfo();
  const api = useApi();
  const admin = useIsAdmin();
  const mobile = useIsMobile();
  const { platform, setPlatform, os } = useKeyboard();
  const [enter, setEnter] = useEnterBehavior();
  const [editing, setEditing] = useState(false);
  const diffs = differencesFromQbd('browser', os);
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="company">
      <Group justify="space-between">
        <Title order={3}>Company</Title>
        {admin && (
          <Button size="xs" variant="default" leftSection={<IconPencil size={14} />} onClick={() => setEditing(true)} disabled={!info}>
            Edit company information
          </Button>
        )}
      </Group>
      {isLoading || !info ? (
        <Skeleton h={160} />
      ) : (
        <SimpleGrid cols={{ base: 1, md: 2 }}>
          <Card withBorder>
            <Text fw={600}>{info.name}</Text>
            {info.legal_name && <Text c="dimmed">{info.legal_name}</Text>}
            {(info.address.lines ?? []).map((l) => (
              <Text key={l} size="sm">
                {l}
              </Text>
            ))}
            <Text size="sm">{[info.address.city, info.address.state, info.address.postal_code].filter(Boolean).join(' ')}</Text>
            {info.phone && <Text size="sm">{info.phone}</Text>}
            {info.email && <Text size="sm">{info.email}</Text>}
            {info.website && <Text size="sm">{info.website}</Text>}
          </Card>
          <Card withBorder>
            <Table fz="sm">
              <Table.Tbody>
                <Table.Tr>
                  <Table.Td c="dimmed">Fiscal year starts</Table.Td>
                  <Table.Td>{MONTHS[info.fiscal_year_start_month - 1]?.label}</Table.Td>
                </Table.Tr>
                <Table.Tr>
                  <Table.Td c="dimmed">Tax form</Table.Td>
                  <Table.Td>{info.tax_form ?? '—'}</Table.Td>
                </Table.Tr>
                <Table.Tr>
                  <Table.Td c="dimmed">EIN</Table.Td>
                  <Table.Td>{info.ein ?? '—'}</Table.Td>
                </Table.Tr>
                <Table.Tr>
                  <Table.Td c="dimmed">Class tracking</Table.Td>
                  <Table.Td>{info.preferences.use_class_tracking ? 'On' : 'Off'}</Table.Td>
                </Table.Tr>
                <Table.Tr>
                  <Table.Td c="dimmed">Data source</Table.Td>
                  <Table.Td>{api.mode === 'fixtures' ? 'Fixture mode (synthetic, in memory)' : 'Server'}</Table.Td>
                </Table.Tr>
              </Table.Tbody>
            </Table>
          </Card>
        </SimpleGrid>
      )}

      <ClosingDateCard admin={admin} />

      {admin && <AdminLinks />}

      <Title order={4}>My preferences (this device)</Title>
      <Card withBorder>
        <Stack gap="md">
          <div>
            <Text fw={500}>Keyboard shortcuts</Text>
            <Text size="sm" c="dimmed" mb="xs">
              The desktop app uses QuickBooks Desktop's keys. A browser keeps some keys for itself (Ctrl+W closes the tab), so the browser keymap uses alternates for those.
            </Text>
            <SegmentedControl
              data={[
                { value: 'browser', label: 'Browser keymap' },
                { value: 'desktop', label: 'Desktop app (QBD) keymap' },
              ]}
              value={platform}
              onChange={(v) => setPlatform(v as 'browser' | 'desktop')}
            />
            {diffs.length > 0 && !mobile && (
              <Table fz="xs" mt="sm" maw={560}>
                <Table.Thead>
                  <Table.Tr>
                    <Table.Th>Command</Table.Th>
                    <Table.Th>QuickBooks</Table.Th>
                    <Table.Th>In a browser</Table.Th>
                  </Table.Tr>
                </Table.Thead>
                <Table.Tbody>
                  {diffs.map((d) => (
                    <Table.Tr key={d.command}>
                      <Table.Td>{COMMANDS[d.command].label}</Table.Td>
                      <Table.Td>{d.qbd.map((c) => formatCombo(c, os)).join(', ')}</Table.Td>
                      <Table.Td>{d.here.map((c) => formatCombo(c, os)).join(', ')}</Table.Td>
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            )}
          </div>
          <div>
            <Text fw={500}>Enter key in line grids</Text>
            <Group gap="xs" mt="xs">
              <SegmentedControl
                data={[
                  { value: 'next-cell', label: 'Next field' },
                  { value: 'next-row', label: 'Next line' },
                  { value: 'save', label: 'Save the form' },
                ]}
                value={enter}
                onChange={(v) => setEnter(v as EnterBehavior)}
              />
            </Group>
          </div>
        </Stack>
      </Card>
      {info && <EditInfoModal opened={editing} info={info} onClose={() => setEditing(false)} />}
    </Stack>
  );
}

/** QBD's Set Closing Date: transactions on or before it are locked. */
function ClosingDateCard({ admin }: { admin: boolean }) {
  const api = useApi();
  const co = useCompanyId();
  const invalidate = useInvalidate();
  const closing = useClosingDate();
  const [date, setDate] = useState<string | null>(null);
  const [touched, setTouched] = useState(false);
  useEffect(() => {
    if (!touched) setDate(closing.data ?? null);
  }, [closing.data, touched]);
  const save = useMutation({
    mutationFn: (d: string | null) => api.putClosingDate(co, d),
    onSuccess: (d) => {
      invalidate(qk.closingDate(co));
      setTouched(false);
      notifications.show({ message: d ? `Books closed through ${formatDate(d)}.` : 'Closing date removed.', color: 'teal' });
    },
  });
  return (
    <Card withBorder data-testid="closing-date">
      <Group justify="space-between" align="flex-start" wrap="wrap">
        <div>
          <Group gap="xs">
            <IconLock size={16} />
            <Text fw={600}>Closing date</Text>
          </Group>
          <Text size="sm" c="dimmed">
            {closing.data ? `The books are closed through ${formatDate(closing.data)}: nothing dated on or before it can be added, changed or deleted.` : 'No closing date. Set one after the year is filed so past periods stay as they were reported.'}
          </Text>
        </div>
        {admin && (
          <Group align="flex-end" gap="xs">
            <DateField
              label="Closed through"
              value={date}
              onChange={(v) => {
                setDate(v);
                setTouched(true);
              }}
              aria-label="Closing date"
            />
            <Button size="sm" onClick={() => save.mutate(date)} disabled={!touched || date === (closing.data ?? null)} loading={save.isPending}>
              Set
            </Button>
            {closing.data && (
              <Button size="sm" variant="subtle" onClick={() => save.mutate(null)} loading={save.isPending}>
                Remove
              </Button>
            )}
          </Group>
        )}
      </Group>
      {save.error && (
        <Alert color="red" mt="sm">
          {describeError(save.error)}
        </Alert>
      )}
    </Card>
  );
}

function AdminLink({ icon, label, hint, path }: { icon: ReactNode; label: string; hint: string; path: string }) {
  const open = useOpenWindow();
  return (
    <UnstyledButton className={classes.action} onClick={() => open(path)} aria-label={label}>
      <div className={classes.actionIcon}>{icon}</div>
      <Text size="sm" fw={500}>
        {label}
      </Text>
      <Text size="xs" c="dimmed" ta="center">
        {hint}
      </Text>
    </UnstyledButton>
  );
}

function AdminLinks() {
  return (
    <div>
      <Title order={4} mb="xs">
        Administration
      </Title>
      <SimpleGrid cols={{ base: 2, sm: 4 }} spacing="xs">
        <AdminLink icon={<IconUsers size={22} />} label="Users and roles" hint="who can sign in" path={paths.users()} />
        <AdminLink icon={<IconLink size={22} />} label="Enrolment links" hint="add a person or device" path={paths.enrollments()} />
        <AdminLink icon={<IconServer size={22} />} label="Server settings" hint="updates, tunnel, backups" path={paths.server()} />
        <AdminLink icon={<IconDatabaseExport size={22} />} label="Backups" hint="this company's copies" path={paths.backups()} />
        <AdminLink icon={<IconCloudUpload size={22} />} label="Import from QuickBooks" hint="sync and parity" path={paths.qbdImport()} />
        <AdminLink icon={<IconArrowsJoin size={22} />} label="Merge" hint="fold duplicates together" path={paths.merge()} />
        <AdminLink icon={<IconPaperclip size={22} />} label="Attachments" hint="receipts and documents" path={paths.attachments()} />
        <AdminLink icon={<IconFileImport size={22} />} label="Memorized transactions" hint="Ctrl+T" path={paths.memorized()} />
      </SimpleGrid>
    </div>
  );
}

function EditInfoModal({ opened, info, onClose }: { opened: boolean; info: CompanyInfo; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const invalidate = useInvalidate();
  const [draft, setDraft] = useState<CompanyInfo>(info);
  useEffect(() => {
    if (opened) setDraft(structuredClone(info));
  }, [opened, info]);
  const save = useMutation({
    mutationFn: () => api.putCompanyInfo(co, draft),
    onSuccess: () => {
      invalidate(qk.info(co));
      notifications.show({ message: 'Company information saved.', color: 'teal' });
      onClose();
    },
  });
  const field = (name: string) => (isApiError(save.error) && (save.error.detail as { field?: string } | undefined)?.field === name ? save.error.message : undefined);
  return (
    <Modal opened={opened} onClose={onClose} title="Company information" size="lg" centered>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <Stack gap="sm">
          <CompanyInfoFields draft={draft} onChange={setDraft} fieldError={field} />
          {save.error && !field('name') && !field('fiscal_year_start_month') && <Alert color="red">{describeError(save.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" loading={save.isPending}>
              Save
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}
