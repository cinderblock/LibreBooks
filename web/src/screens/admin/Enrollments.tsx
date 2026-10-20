// Enrolment links (Company → Users → Enrolment links): the outstanding
// one-time tokens, and minting a new one. A minted link is shown exactly once,
// as text to copy and as a QR code for a phone; the server never stores the
// token, so closing the dialog is the end of it.

import { ActionIcon, Alert, Badge, Button, Card, CopyButton, Group, Modal, NumberInput, SegmentedControl, Select, Stack, Table, Text, TextInput, Title, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconCheck, IconCopy, IconLink, IconTrash } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { QRCodeSVG } from 'qrcode.react';
import { useEffect, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useEnrollments, useInvalidate, useIsAdmin, useSession, useUsers } from '~/api/hooks';
import { sk } from '~/api/keys';
import type { EnrolmentPurpose, MintedEnrolmentLink, NewEnrolment, Role } from '~/api/types';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow } from '~/windows/WindowsProvider';
import classes from '../screens.module.css';
import { AdminGate, Confirm, ROLE_LABEL, RoleSelect, When } from './common';

export const PURPOSE_LABEL: Record<EnrolmentPurpose, string> = { first_admin: 'First admin', new_user: 'New user', add_device: 'Add a device' };

export function EnrollmentsWindow() {
  const win = useWindow();
  useEffect(() => win.setTitle('Enrolment Links'), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <AdminGate what="Minting enrolment links">
      <Enrollments />
    </AdminGate>
  );
}

function Enrollments() {
  const api = useApi();
  const mobile = useIsMobile();
  const list = useEnrollments();
  const users = useUsers();
  const invalidate = useInvalidate();
  const [minting, setMinting] = useState(false);
  const [revoking, setRevoking] = useState<string | null>(null);
  const names = new Map((users.data ?? []).map((u) => [u.id, u.display_name]));
  const revoke = useMutation({
    mutationFn: (id: string) => api.revokeEnrollment(id),
    onSuccess: () => {
      invalidate(sk.enrollments);
      setRevoking(null);
      notifications.show({ message: 'Link revoked.', color: 'teal' });
    },
  });
  const rows = list.data ?? [];
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="enrollments">
      <Group justify="space-between">
        <div>
          <Title order={3}>Enrolment links</Title>
          <Text size="sm" c="dimmed">
            A link lets one person set up a passkey, once, before it expires. Share it any way you trust; nothing else is needed.
          </Text>
        </div>
        <Button leftSection={<IconLink size={16} />} onClick={() => setMinting(true)}>
          New link
        </Button>
      </Group>
      {rows.length === 0 && !list.isLoading && (
        <Text c="dimmed" size="sm">
          No links outstanding. Links disappear from here when they're used or expire.
        </Text>
      )}
      {rows.length > 0 && (
        <Table fz="sm" withTableBorder>
          <Table.Thead>
            <Table.Tr>
              <Table.Th>For</Table.Th>
              <Table.Th>Purpose</Table.Th>
              {!mobile && <Table.Th>Role</Table.Th>}
              {!mobile && <Table.Th>Made</Table.Th>}
              <Table.Th>Expires</Table.Th>
              <Table.Th />
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {rows.map((e) => (
              <Table.Tr key={e.id} data-testid="enrollment-row">
                <Table.Td>{e.purpose === 'add_device' ? (names.get(e.user_id ?? '') ?? e.user_id) : (e.display_name ?? <Text span c="dimmed">(name chosen when enrolling)</Text>)}</Table.Td>
                <Table.Td>
                  <Badge variant="light" color={e.purpose === 'add_device' ? 'grape' : 'teal'}>
                    {PURPOSE_LABEL[e.purpose]}
                  </Badge>
                </Table.Td>
                {!mobile && <Table.Td>{e.role ? ROLE_LABEL[e.role] : '—'}</Table.Td>}
                {!mobile && (
                  <Table.Td>
                    <When iso={e.created_at} />
                    {e.created_by && <Text span c="dimmed"> by {names.get(e.created_by) ?? 'an admin'}</Text>}
                  </Table.Td>
                )}
                <Table.Td>
                  <When iso={e.expires_at} />
                </Table.Td>
                <Table.Td ta="right">
                  <Tooltip label="Revoke">
                    <ActionIcon variant="subtle" color="red" aria-label={`Revoke link for ${e.display_name ?? names.get(e.user_id ?? '') ?? 'user'}`} onClick={() => setRevoking(e.id)}>
                      <IconTrash size={16} />
                    </ActionIcon>
                  </Tooltip>
                </Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      )}
      <MintLinkModal opened={minting} onClose={() => setMinting(false)} />
      <Confirm opened={!!revoking} title="Revoke this link?" confirmLabel="Revoke" loading={revoke.isPending} onConfirm={() => revoking && revoke.mutate(revoking)} onClose={() => setRevoking(null)}>
        <Text size="sm">Anyone holding the link will find it no longer works. You can mint another.</Text>
        {revoke.error && <Alert color="red" mt="sm">{describeError(revoke.error)}</Alert>}
      </Confirm>
    </Stack>
  );
}

/**
 * Mint a link. `purpose` fixed by the caller (My Account offers "add a device"
 * for yourself only); otherwise the admin picks.
 */
export function MintLinkModal({ opened, onClose, fixed }: { opened: boolean; onClose(): void; fixed?: { purpose: 'add_device'; userId?: string } }) {
  const api = useApi();
  const co = useCompanyId();
  const admin = useIsAdmin();
  const session = useSession();
  const users = useUsers(admin && opened && !fixed);
  const invalidate = useInvalidate();
  const [purpose, setPurpose] = useState<'new_user' | 'add_device'>(fixed?.purpose ?? 'new_user');
  const [role, setRole] = useState<Role>('bookkeeper');
  const [name, setName] = useState('');
  const [userId, setUserId] = useState<string | null>(fixed?.userId ?? null);
  const [ttl, setTtl] = useState(15);
  const [grant, setGrant] = useState(true);
  const [minted, setMinted] = useState<MintedEnrolmentLink | null>(null);
  useEffect(() => {
    if (!opened) return;
    setMinted(null);
    setPurpose(fixed?.purpose ?? 'new_user');
    setUserId(fixed?.userId ?? session.data?.user.id ?? null);
    setName('');
  }, [opened, fixed, session.data?.user.id]);
  const mint = useMutation({
    mutationFn: (body: NewEnrolment) => api.createEnrollment(body),
    onSuccess: (m) => {
      setMinted(m);
      invalidate(sk.enrollments);
    },
  });
  const submit = () => {
    const body: NewEnrolment =
      purpose === 'new_user'
        ? { purpose, role, display_name: name.trim() || undefined, company_id: grant ? co : undefined, ttl_minutes: ttl }
        : { purpose, user_id: userId ?? undefined, ttl_minutes: ttl };
    mint.mutate(body);
  };
  const userData = (users.data ?? []).filter((u) => !u.disabled_at).map((u) => ({ value: u.id, label: u.display_name }));
  return (
    <Modal opened={opened} onClose={onClose} title={minted ? 'Share this link' : fixed ? 'Add a device' : 'New enrolment link'} centered size={minted ? 'md' : 'sm'}>
      {minted ? (
        <MintedLink link={minted} onDone={onClose} />
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <Stack gap="sm">
            {!fixed && (
              <SegmentedControl
                fullWidth
                data={[
                  { value: 'new_user', label: 'New user' },
                  { value: 'add_device', label: 'Add a device' },
                ]}
                value={purpose}
                onChange={(v) => setPurpose(v as 'new_user' | 'add_device')}
              />
            )}
            {purpose === 'new_user' ? (
              <>
                <TextInput label="Their name" description="A suggestion; they can change it when enrolling." value={name} onChange={(e) => setName(e.currentTarget.value)} data-autofocus />
                <RoleSelect value={role} onChange={setRole} />
                <SegmentedControl
                  fullWidth
                  data={[
                    { value: 'yes', label: 'Access to this company' },
                    { value: 'no', label: 'No company yet' },
                  ]}
                  value={grant ? 'yes' : 'no'}
                  onChange={(v) => setGrant(v === 'yes')}
                />
              </>
            ) : (
              !fixed && admin && <Select label="Whose device" data={userData} value={userId} onChange={setUserId} searchable allowDeselect={false} description="The link signs that person's new phone or computer in with its own passkey." />
            )}
            {fixed && (
              <Text size="sm" c="dimmed">
                Open the link on the new device and it gets a passkey of its own for your account.
              </Text>
            )}
            <NumberInput label="Good for (minutes)" value={ttl} onChange={(v) => setTtl(Math.max(1, Number(v) || 15))} min={1} max={1440} />
            {mint.error && <Alert color="red">{describeError(mint.error)}</Alert>}
            <Group justify="flex-end">
              <Button variant="default" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" loading={mint.isPending}>
                Mint link
              </Button>
            </Group>
          </Stack>
        </form>
      )}
    </Modal>
  );
}

/** The one showing of a minted link: text, copy button, QR. */
export function MintedLink({ link, onDone }: { link: MintedEnrolmentLink; onDone(): void }) {
  return (
    <Stack gap="md" align="center" data-testid="minted-link">
      <Card withBorder p="sm" bg="white">
        <QRCodeSVG value={link.url} size={200} marginSize={1} aria-label="Enrolment link as a QR code" />
      </Card>
      <Group gap="xs" wrap="nowrap" w="100%">
        <TextInput value={link.url} readOnly style={{ flex: 1 }} aria-label="Enrolment link" onFocus={(e) => e.currentTarget.select()} />
        <CopyButton value={link.url}>
          {({ copied, copy }) => (
            <Button variant={copied ? 'light' : 'default'} color={copied ? 'teal' : undefined} leftSection={copied ? <IconCheck size={16} /> : <IconCopy size={16} />} onClick={copy}>
              {copied ? 'Copied' : 'Copy'}
            </Button>
          )}
        </CopyButton>
      </Group>
      <Text size="sm" c="dimmed" ta="center">
        {PURPOSE_LABEL[link.purpose]} · expires <When iso={link.expires_at} />. This is the only time the link is shown; it's not stored anywhere.
      </Text>
      <Button onClick={onDone}>Done</Button>
    </Stack>
  );
}
