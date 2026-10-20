// My Account (the avatar menu): who I am, my passkeys (rename, remove), my
// device keys (revoke), where I'm signed in (sign other devices out), and the
// two ways to add a sign-in: a link for another device, or a passkey on this
// one (which mints the same kind of link and follows it here).

import { ActionIcon, Alert, Badge, Button, Card, Group, Modal, Stack, Table, Text, TextInput, Title, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconDeviceMobile, IconKey, IconLink, IconLogout, IconPencil, IconTrash } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { useApi } from '~/api/context';
import { describeError } from '~/api/errors';
import { useDevices, useInvalidate, usePasskeys, useSession, useSessions } from '~/api/hooks';
import { sk } from '~/api/keys';
import type { PasskeyInfo, SessionInfo } from '~/api/types';
import { useIsMobile } from '~/lib/useMobile';
import { useSignOut } from '~/shell/UserMenu';
import { useWindow } from '~/windows/WindowsProvider';
import { MintLinkModal } from './admin/Enrollments';
import { Confirm, ROLE_LABEL, When } from './admin/common';
import classes from './screens.module.css';

export function MyAccount() {
  const win = useWindow();
  const api = useApi();
  const mobile = useIsMobile();
  const session = useSession();
  const passkeys = usePasskeys();
  const devices = useDevices();
  const sessions = useSessions();
  const invalidate = useInvalidate();
  const signOut = useSignOut();
  const [renaming, setRenaming] = useState<PasskeyInfo | null>(null);
  const [removing, setRemoving] = useState<{ kind: 'passkey' | 'device' | 'session'; id: string; label: string } | null>(null);
  const [minting, setMinting] = useState(false);
  useEffect(() => win.setTitle('My Account'), []); // eslint-disable-line react-hooks/exhaustive-deps

  const remove = useMutation({
    mutationFn: ({ kind, id }: { kind: 'passkey' | 'device' | 'session'; id: string }) => (kind === 'passkey' ? api.deletePasskey(id) : kind === 'device' ? api.revokeDevice(id) : api.revokeSession(id)),
    onSuccess: (_v, { kind }) => {
      invalidate(sk.me);
      setRemoving(null);
      notifications.show({ message: kind === 'session' ? 'Signed that device out.' : kind === 'passkey' ? 'Passkey removed.' : 'Device key revoked.', color: 'teal' });
    },
  });
  // "Add a passkey on this device": mint an add-device link for me and follow it here.
  const addHere = useMutation({
    mutationFn: () => api.createEnrollment({ purpose: 'add_device', ttl_minutes: 5 }),
    onSuccess: (m) => {
      const u = new URL(m.url, window.location.origin);
      window.location.assign(`${u.pathname}${u.hash}`);
    },
  });

  const user = session.data?.user;
  const local = user?.kind === 'local';
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="my-account">
      <div>
        <Title order={3}>{user?.display_name ?? 'My account'}</Title>
        {user && (
          <Text size="sm" c="dimmed">
            {ROLE_LABEL[user.role]}
            {user.kind ? ` · signed in ${user.kind === 'local' ? 'on this computer (desktop app)' : user.kind === 'device' ? 'with a device key' : 'with a passkey'}` : ''}
          </Text>
        )}
      </div>
      {local && (
        <Alert color="blue" variant="light">
          The desktop app signs you in by itself; passkeys and sessions only matter when this server is reached over the network.
        </Alert>
      )}

      <Card withBorder>
        <Group justify="space-between" mb="xs">
          <Group gap="xs">
            <IconKey size={18} />
            <Text fw={600}>Passkeys</Text>
          </Group>
          <Group gap="xs">
            <Button size="xs" variant="default" leftSection={<IconLink size={14} />} onClick={() => setMinting(true)}>
              Add another device
            </Button>
            {!local && (
              <Button size="xs" loading={addHere.isPending} onClick={() => addHere.mutate()}>
                Add a passkey here
              </Button>
            )}
          </Group>
        </Group>
        {addHere.error && <Alert color="red" mb="xs">{describeError(addHere.error)}</Alert>}
        {(passkeys.data ?? []).length === 0 && !passkeys.isLoading && (
          <Text size="sm" c="dimmed">
            No passkeys. {local ? 'None are needed for the desktop app.' : 'Add one before you need to sign in from somewhere else.'}
          </Text>
        )}
        {(passkeys.data ?? []).length > 0 && (
          <Table fz="sm">
            <Table.Tbody>
              {passkeys.data!.map((p) => (
                <Table.Tr key={p.id} data-testid="passkey-row">
                  <Table.Td>
                    <Text size="sm">{p.name}</Text>
                    <Text size="xs" c="dimmed">
                      {p.rp_id}
                      {p.backed_up ? ' · synced' : p.backup_eligible ? ' · can sync' : ' · hardware key'}
                    </Text>
                  </Table.Td>
                  {!mobile && (
                    <Table.Td>
                      <Text size="xs" c="dimmed">
                        added <When iso={p.created_at} />
                      </Text>
                    </Table.Td>
                  )}
                  <Table.Td>
                    <Text size="xs" c="dimmed">
                      used <When iso={p.last_used_at} fallback="never" />
                    </Text>
                  </Table.Td>
                  <Table.Td ta="right">
                    <Group gap={2} justify="flex-end" wrap="nowrap">
                      <Tooltip label="Rename">
                        <ActionIcon variant="subtle" aria-label={`Rename passkey ${p.name}`} onClick={() => setRenaming(p)}>
                          <IconPencil size={16} />
                        </ActionIcon>
                      </Tooltip>
                      <Tooltip label="Remove">
                        <ActionIcon variant="subtle" color="red" aria-label={`Remove passkey ${p.name}`} onClick={() => setRemoving({ kind: 'passkey', id: p.id, label: p.name })}>
                          <IconTrash size={16} />
                        </ActionIcon>
                      </Tooltip>
                    </Group>
                  </Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
        )}
      </Card>

      {(devices.data ?? []).length > 0 && (
        <Card withBorder>
          <Group gap="xs" mb="xs">
            <IconDeviceMobile size={18} />
            <Text fw={600}>Device keys</Text>
            <Text size="xs" c="dimmed">
              thin clients and kiosks signed in with a key of their own
            </Text>
          </Group>
          <Table fz="sm">
            <Table.Tbody>
              {devices.data!.map((d) => (
                <Table.Tr key={d.id}>
                  <Table.Td>{d.name}</Table.Td>
                  <Table.Td>
                    <Text size="xs" c="dimmed">
                      used <When iso={d.last_used_at} fallback="never" />
                    </Text>
                  </Table.Td>
                  <Table.Td ta="right">
                    <Button size="compact-xs" variant="subtle" color="red" onClick={() => setRemoving({ kind: 'device', id: d.id, label: d.name })}>
                      Revoke
                    </Button>
                  </Table.Td>
                </Table.Tr>
              ))}
            </Table.Tbody>
          </Table>
        </Card>
      )}

      <Card withBorder>
        <Group justify="space-between" mb="xs">
          <Text fw={600}>Where you're signed in</Text>
          {signOut && (
            <Button size="xs" variant="default" leftSection={<IconLogout size={14} />} onClick={signOut}>
              Sign out here
            </Button>
          )}
        </Group>
        <Table fz="sm">
          <Table.Tbody>
            {(sessions.data ?? []).map((s) => (
              <SessionRow key={s.id} s={s} mobile={mobile} onRevoke={() => setRemoving({ kind: 'session', id: s.id, label: s.label ?? 'that device' })} />
            ))}
          </Table.Tbody>
        </Table>
      </Card>

      <RenameModal passkey={renaming} onClose={() => setRenaming(null)} />
      <MintLinkModal opened={minting} onClose={() => setMinting(false)} fixed={{ purpose: 'add_device', userId: user?.id ?? undefined }} />
      <Confirm
        opened={!!removing}
        title={removing?.kind === 'session' ? `Sign out ${removing.label}?` : removing?.kind === 'passkey' ? `Remove passkey "${removing.label}"?` : `Revoke "${removing?.label}"?`}
        confirmLabel={removing?.kind === 'session' ? 'Sign out' : removing?.kind === 'passkey' ? 'Remove' : 'Revoke'}
        loading={remove.isPending}
        onConfirm={() => removing && remove.mutate(removing)}
        onClose={() => setRemoving(null)}
      >
        <Text size="sm">
          {removing?.kind === 'session' && "That device is signed out at once; it can sign in again with its passkey."}
          {removing?.kind === 'passkey' && "You won't be able to sign in with it anymore. Keep at least one way in."}
          {removing?.kind === 'device' && 'The device loses access until it enrols again.'}
        </Text>
        {remove.error && <Alert color="red" mt="sm">{describeError(remove.error)}</Alert>}
      </Confirm>
    </Stack>
  );
}

function SessionRow({ s, mobile, onRevoke }: { s: SessionInfo; mobile: boolean; onRevoke(): void }) {
  return (
    <Table.Tr data-testid="session-row">
      <Table.Td>
        <Group gap="xs">
          <Text size="sm">{s.label ?? (s.kind === 'local' ? 'This computer' : 'Unknown device')}</Text>
          {s.current && <Badge size="xs">This device</Badge>}
        </Group>
        <Text size="xs" c="dimmed">
          {s.kind === 'device' ? 'device key' : s.kind === 'local' ? 'desktop app' : 'passkey'}
          {s.ip ? ` · ${s.ip}` : ''}
        </Text>
      </Table.Td>
      {!mobile && (
        <Table.Td>
          <Text size="xs" c="dimmed">
            active <When iso={s.last_seen_at} />
          </Text>
        </Table.Td>
      )}
      <Table.Td>
        <Text size="xs" c="dimmed">
          expires <When iso={s.expires_at} />
        </Text>
      </Table.Td>
      <Table.Td ta="right">
        {!s.current && (
          <Button size="compact-xs" variant="subtle" color="red" onClick={onRevoke}>
            Sign out
          </Button>
        )}
      </Table.Td>
    </Table.Tr>
  );
}

function RenameModal({ passkey, onClose }: { passkey: PasskeyInfo | null; onClose(): void }) {
  const api = useApi();
  const invalidate = useInvalidate();
  const [name, setName] = useState('');
  useEffect(() => setName(passkey?.name ?? ''), [passkey]);
  const rename = useMutation({
    mutationFn: () => api.renamePasskey(passkey!.id, name.trim()),
    onSuccess: () => {
      invalidate(sk.passkeys);
      onClose();
    },
  });
  return (
    <Modal opened={!!passkey} onClose={onClose} title="Rename passkey" centered size="sm">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          rename.mutate();
        }}
      >
        <Stack gap="sm">
          <TextInput label="Name" value={name} onChange={(e) => setName(e.currentTarget.value)} data-autofocus required />
          {rename.error && <Alert color="red">{describeError(rename.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" loading={rename.isPending} disabled={!name.trim()}>
              Save
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}
