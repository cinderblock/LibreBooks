// Bits the administrative windows share: the admin gate (a viewer or
// bookkeeper who lands on an admin window is told so, not shown a broken
// screen), role labels, a confirm dialog, and the little "when" text.

import { Alert, Button, Group, Modal, Select, Stack, Text, Tooltip } from '@mantine/core';
import type { ReactNode } from 'react';
import { useIsAdmin, useSession } from '~/api/hooks';
import type { Role } from '~/api/types';
import { formatDateTime, relativeTime } from '~/lib/dates';

export const ROLE_LABEL: Record<Role, string> = { viewer: 'Viewer', bookkeeper: 'Bookkeeper', admin: 'Admin' };

export const ROLE_HELP: Record<Role, string> = {
  viewer: 'Reads everything, changes nothing.',
  bookkeeper: 'Enters and edits transactions and lists.',
  admin: 'Everything, plus users, settings and the closing date.',
};

const ROLE_DATA = (Object.keys(ROLE_LABEL) as Role[]).map((r) => ({ value: r, label: ROLE_LABEL[r] }));

export function RoleSelect({ value, onChange, label = 'Role', disabled, size }: { value: Role; onChange(r: Role): void; label?: ReactNode; disabled?: boolean; size?: 'xs' | 'sm' }) {
  return <Select label={label} data={ROLE_DATA} value={value} onChange={(v) => v && onChange(v as Role)} allowDeselect={false} disabled={disabled} size={size} description={label ? ROLE_HELP[value] : undefined} />;
}

/**
 * Admin-only windows render through this. The server refuses anyway (403); the
 * gate just says so plainly before any request is made.
 */
export function AdminGate({ children, what }: { children: ReactNode; what: string }) {
  const session = useSession();
  const admin = useIsAdmin();
  if (session.isLoading) return null;
  if (!admin) {
    return (
      <Stack p="xl" maw={560}>
        <Alert color="yellow" title="Admins only">
          {what} is for server admins. Ask one of yours ({session.data?.user.display_name ? `you're signed in as ${session.data.user.display_name}` : 'you are not an admin'}) if something here needs changing.
        </Alert>
      </Stack>
    );
  }
  return <>{children}</>;
}

/** "3 hours ago" with the exact time on hover. */
export function When({ iso, fallback = '—' }: { iso: string | null | undefined; fallback?: string }) {
  if (!iso) return <Text span c="dimmed">{fallback}</Text>;
  return (
    <Tooltip label={formatDateTime(iso)} openDelay={400}>
      <Text span>{relativeTime(iso)}</Text>
    </Tooltip>
  );
}

export interface ConfirmProps {
  opened: boolean;
  title: string;
  children: ReactNode;
  confirmLabel?: string;
  color?: string;
  loading?: boolean;
  onConfirm(): void;
  onClose(): void;
}

/** A yes/no dialog for the destructive buttons (revoke, delete, restore…). */
export function Confirm({ opened, title, children, confirmLabel = 'Yes', color = 'red', loading, onConfirm, onClose }: ConfirmProps) {
  return (
    <Modal opened={opened} onClose={onClose} title={title} centered>
      <Stack gap="md">
        <div>{children}</div>
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose} disabled={loading}>
            Cancel
          </Button>
          <Button color={color} onClick={onConfirm} loading={loading} data-autofocus>
            {confirmLabel}
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
