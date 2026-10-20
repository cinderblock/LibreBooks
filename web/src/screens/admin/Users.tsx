// Users (Company → Users and Roles): everyone who can sign in, their
// server-wide role, whether they're disabled, and their access to this
// company. Selecting a user opens the detail panel: rename, change role,
// disable/enable, grant or revoke this company, see passkeys and devices,
// mint an add-device link, delete.

import { Alert, Badge, Button, Divider, Drawer, Group, Select, Stack, Switch, Table, Text, TextInput, Title } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconDeviceMobile, IconKey, IconLink, IconUserPlus } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useCompanyInfo, useInvalidate, useSession, useUser, useUsers } from '~/api/hooks';
import { sk } from '~/api/keys';
import type { Role, User, UserUpdate } from '~/api/types';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from '../screens.module.css';
import { AdminGate, Confirm, ROLE_LABEL, RoleSelect, When } from './common';
import { MintLinkModal } from './Enrollments';

export function UsersWindow() {
  const win = useWindow();
  useEffect(() => win.setTitle('Users and Roles'), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <AdminGate what="Managing users">
      <Users />
    </AdminGate>
  );
}

function Users() {
  const mobile = useIsMobile();
  const users = useUsers();
  const open = useOpenWindow();
  const [selected, setSelected] = useState<string | null>(null);
  const [minting, setMinting] = useState(false);
  const rows = users.data ?? [];
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="users">
      <Group justify="space-between">
        <div>
          <Title order={3}>Users and roles</Title>
          <Text size="sm" c="dimmed">
            People sign in with a passkey. Add someone by minting an enrolment link for them.
          </Text>
        </div>
        <Group gap="xs">
          <Button variant="default" leftSection={<IconLink size={16} />} onClick={() => open(paths.enrollments())}>
            Enrolment links
          </Button>
          <Button leftSection={<IconUserPlus size={16} />} onClick={() => setMinting(true)}>
            Add a user
          </Button>
        </Group>
      </Group>
      <Table fz="sm" withTableBorder highlightOnHover className={classes.clickable}>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Name</Table.Th>
            <Table.Th>Role</Table.Th>
            {!mobile && <Table.Th>Since</Table.Th>}
            <Table.Th>Status</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {rows.map((u) => (
            <Table.Tr key={u.id} onClick={() => setSelected(u.id)} data-testid="user-row" tabIndex={0} onKeyDown={(e) => e.key === 'Enter' && setSelected(u.id)} aria-label={`${u.display_name}, ${ROLE_LABEL[u.role]}`}>
              <Table.Td>{u.display_name}</Table.Td>
              <Table.Td>
                <Badge variant="light" color={u.role === 'admin' ? 'grape' : u.role === 'bookkeeper' ? 'teal' : 'gray'}>
                  {ROLE_LABEL[u.role]}
                </Badge>
              </Table.Td>
              {!mobile && (
                <Table.Td>
                  <When iso={u.created_at} />
                </Table.Td>
              )}
              <Table.Td>{u.disabled_at ? <Badge color="red" variant="light">Disabled</Badge> : <Text span c="dimmed">Active</Text>}</Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
      <UserDrawer id={selected} onClose={() => setSelected(null)} />
      <MintLinkModal opened={minting} onClose={() => setMinting(false)} />
    </Stack>
  );
}

function UserDrawer({ id, onClose }: { id: string | null; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const session = useSession();
  const info = useCompanyInfo();
  const detail = useUser(id);
  const invalidate = useInvalidate();
  const [name, setName] = useState('');
  const [role, setRole] = useState<Role>('bookkeeper');
  const [confirm, setConfirm] = useState<'delete' | 'disable' | null>(null);
  const [minting, setMinting] = useState(false);
  const user = detail.data?.user;
  const me = session.data?.user.id === id;
  useEffect(() => {
    if (user) {
      setName(user.display_name);
      setRole(user.role);
    }
  }, [user]);
  const refresh = () => {
    invalidate(sk.users, sk.user(id ?? ''));
  };
  const update = useMutation({
    mutationFn: (u: UserUpdate) => api.updateUser(id!, u),
    onSuccess: (u: User) => {
      refresh();
      setConfirm(null);
      notifications.show({ message: `${u.display_name} saved.`, color: 'teal' });
    },
  });
  const remove = useMutation({
    mutationFn: () => api.deleteUser(id!),
    onSuccess: () => {
      refresh();
      setConfirm(null);
      onClose();
      notifications.show({ message: 'User deleted.', color: 'teal' });
    },
  });
  const access = detail.data?.companies.find((c) => c.company_id === co) ?? null;
  const grant = useMutation({
    mutationFn: (r: Role | null) => (r ? api.grantAccess(id!, co, r) : api.revokeAccess(id!, co)),
    onSuccess: refresh,
  });
  const dirty = !!user && (name.trim() !== user.display_name || role !== user.role);
  return (
    <Drawer opened={!!id} onClose={onClose} position="right" size={mobile ? '100%' : 'md'} title={user?.display_name ?? 'User'}>
      {detail.isLoading && <Text c="dimmed">Loading…</Text>}
      {detail.error && <Alert color="red">{describeError(detail.error)}</Alert>}
      {user && (
        <Stack gap="md" data-testid="user-detail">
          <TextInput label="Name" value={name} onChange={(e) => setName(e.currentTarget.value)} />
          <RoleSelect label="Server role" value={role} onChange={setRole} disabled={me} />
          {me && (
            <Text size="xs" c="dimmed">
              You can't change your own role or disable yourself; another admin can.
            </Text>
          )}
          <Group justify="space-between">
            <Switch
              label={user.disabled_at ? 'Disabled (cannot sign in)' : 'Enabled'}
              checked={!user.disabled_at}
              disabled={me}
              onChange={(e) => (e.currentTarget.checked ? update.mutate({ disabled: false }) : setConfirm('disable'))}
            />
            <Button size="xs" disabled={!dirty} loading={update.isPending} onClick={() => update.mutate({ display_name: name.trim(), ...(role !== user.role ? { role } : {}) })}>
              Save
            </Button>
          </Group>
          {update.error && <Alert color="red">{describeError(update.error)}</Alert>}

          <Divider label={info.data?.name ?? 'This company'} labelPosition="left" />
          <Group align="flex-end" gap="xs">
            <Select
              label="Access to this company"
              data={[{ value: 'none', label: 'No access' }, ...(Object.keys(ROLE_LABEL) as Role[]).map((r) => ({ value: r, label: ROLE_LABEL[r] }))]}
              value={access?.role ?? 'none'}
              onChange={(v) => v && grant.mutate(v === 'none' ? null : (v as Role))}
              allowDeselect={false}
              style={{ flex: 1 }}
              disabled={grant.isPending}
            />
          </Group>
          {detail.data && detail.data.companies.length > 1 && (
            <Text size="xs" c="dimmed">
              Also has access to {detail.data.companies.length - (access ? 1 : 0)} other {detail.data.companies.length - (access ? 1 : 0) === 1 ? 'company' : 'companies'}.
            </Text>
          )}
          {grant.error && <Alert color="red">{describeError(grant.error)}</Alert>}

          <Divider label="Sign-in" labelPosition="left" />
          <Stack gap={4}>
            {detail.data!.passkeys.map((p) => (
              <Group key={p.id} gap="xs" wrap="nowrap">
                <IconKey size={14} />
                <Text size="sm" style={{ flex: 1 }}>
                  {p.name}
                </Text>
                <Text size="xs" c="dimmed">
                  used <When iso={p.last_used_at} fallback="never" />
                </Text>
              </Group>
            ))}
            {detail.data!.devices.map((d) => (
              <Group key={d.id} gap="xs" wrap="nowrap">
                <IconDeviceMobile size={14} />
                <Text size="sm" style={{ flex: 1 }}>
                  {d.name}
                </Text>
                <Text size="xs" c="dimmed">
                  device key · used <When iso={d.last_used_at} fallback="never" />
                </Text>
              </Group>
            ))}
            {detail.data!.passkeys.length + detail.data!.devices.length === 0 && (
              <Text size="sm" c="dimmed">
                No passkeys or device keys yet; they can't sign in until they enrol.
              </Text>
            )}
          </Stack>
          <Button variant="default" size="xs" leftSection={<IconLink size={14} />} onClick={() => setMinting(true)} w="fit-content">
            Link to add a device for {me ? 'yourself' : user.display_name}
          </Button>

          {!me && (
            <>
              <Divider />
              <Button variant="subtle" color="red" size="xs" w="fit-content" onClick={() => setConfirm('delete')}>
                Delete user
              </Button>
            </>
          )}
          <MintLinkModal opened={minting} onClose={() => setMinting(false)} fixed={{ purpose: 'add_device', userId: user.id }} />
          <Confirm
            opened={confirm === 'disable'}
            title={`Disable ${user.display_name}?`}
            confirmLabel="Disable"
            loading={update.isPending}
            onConfirm={() => update.mutate({ disabled: true })}
            onClose={() => setConfirm(null)}
          >
            <Text size="sm">Their sessions end now and they can't sign in until re-enabled. Their passkeys stay.</Text>
          </Confirm>
          <Confirm opened={confirm === 'delete'} title={`Delete ${user.display_name}?`} confirmLabel="Delete" loading={remove.isPending} onConfirm={() => remove.mutate()} onClose={() => setConfirm(null)}>
            <Text size="sm">Removes the account, its passkeys and device keys. What they entered in the books stays, attributed to their name.</Text>
            {remove.error && <Alert color="red" mt="sm">{describeError(remove.error)}</Alert>}
          </Confirm>
        </Stack>
      )}
    </Drawer>
  );
}
