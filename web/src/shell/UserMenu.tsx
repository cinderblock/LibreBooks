// Who's signed in, and signing out (only offered when the server uses sign-in;
// the desktop app's local session and fixture mode have nothing to sign out of).

import { Avatar, Menu, Text, UnstyledButton } from '@mantine/core';
import { IconLogout } from '@tabler/icons-react';
import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from 'react-router';
import { useApi } from '~/api/context';
import { useSession } from '~/api/hooks';

export function useSignOut(): (() => void) | null {
  const api = useApi();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { data } = useSession();
  if (!data?.sign_in) return null;
  return () => {
    void api.logout().finally(() => {
      qc.clear();
      void navigate('/signin', { replace: true });
    });
  };
}

function initials(name: string): string {
  return name
    .split(/[\s,]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((p) => p[0]!.toUpperCase())
    .join('');
}

export function UserMenu() {
  const { data } = useSession();
  const signOut = useSignOut();
  if (!data) return null;
  const { user } = data;
  return (
    <Menu position="bottom-end" withinPortal>
      <Menu.Target>
        <UnstyledButton aria-label={`Signed in as ${user.display_name}`}>
          <Avatar size={26} radius="xl" color="teal">
            {initials(user.display_name)}
          </Avatar>
        </UnstyledButton>
      </Menu.Target>
      <Menu.Dropdown>
        <Menu.Label>
          <Text size="sm" fw={600} c="var(--mantine-color-text)">
            {user.display_name}
          </Text>
          <Text size="xs" c="dimmed">
            {user.role}
            {user.kind ? ` · ${user.kind === 'local' ? 'this computer' : `${user.kind} sign-in`}` : ''}
          </Text>
        </Menu.Label>
        {signOut && (
          <Menu.Item leftSection={<IconLogout size={14} />} onClick={signOut}>
            Sign out
          </Menu.Item>
        )}
      </Menu.Dropdown>
    </Menu>
  );
}
