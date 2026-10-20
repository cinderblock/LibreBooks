// Pieces every transaction form shares: the action bar (with each button's
// shortcut), the "someone else changed this" conflict dialog and banner, and the
// Transaction Journal (Ctrl+Y) and History (Ctrl+H) dialogs.

import { Alert, Button, Group, Kbd, Loader, Modal, Stack, Table, Text, Tooltip, type ButtonProps } from '@mantine/core';
import { IconAlertTriangle, IconRefresh } from '@tabler/icons-react';
import { useQuery } from '@tanstack/react-query';
import type { ComponentPropsWithoutRef, ReactNode } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { Txn } from '~/api/types';
import { formatDate } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import type { CommandId } from '~/keyboard/commands';
import { useComboHint } from '~/keyboard/KeyboardProvider';
import classes from './forms.module.css';

/**
 * Blur the focused field so it commits what was typed (an amount field
 * evaluates its arithmetic on blur), then wait a tick for React to apply it.
 * Saving from a shortcut (Alt+S) while the caret is still in a field needs this.
 */
export async function flushActiveField(): Promise<HTMLElement | null> {
  const el = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  el?.blur();
  await new Promise((r) => setTimeout(r, 0));
  return el;
}

export function ShortcutButton({
  command,
  children,
  ...props
}: { command?: CommandId; children: ReactNode } & ButtonProps & Omit<ComponentPropsWithoutRef<'button'>, keyof ButtonProps>) {
  const hint = useComboHint(command ?? 'palette.open');
  const shown = command ? hint : '';
  const button = (
    <Button {...props} rightSection={shown && !props.rightSection ? <Kbd size="xs">{shown}</Kbd> : props.rightSection}>
      {children}
    </Button>
  );
  return shown ? (
    <Tooltip label={`${typeof children === 'string' ? children : ''} (${shown})`} openDelay={600}>
      {button}
    </Tooltip>
  ) : (
    button
  );
}

export function FormActionBar({ children, mobile }: { children: ReactNode; mobile?: boolean }) {
  return (
    <div className={mobile ? classes.mobileActions : classes.actions}>
      <Group justify="flex-end" gap="xs" wrap={mobile ? 'nowrap' : 'wrap'}>
        {children}
      </Group>
    </div>
  );
}

export function ConflictModal({
  opened,
  what,
  onReload,
  onOverwrite,
  onCancel,
}: {
  opened: boolean;
  what: string;
  onReload(): void;
  onOverwrite(): void;
  onCancel(): void;
}) {
  return (
    <Modal opened={opened} onClose={onCancel} title="Someone else changed this" centered>
      <Stack>
        <Text>
          Another user (or another window) saved this {what} after you opened it. Saving now would overwrite their changes.
        </Text>
        <Text size="sm" c="dimmed">
          Reload to see their version (your edits here are discarded), or keep your version and replace theirs.
        </Text>
        <Group justify="flex-end">
          <Button variant="default" onClick={onCancel}>
            Keep editing
          </Button>
          <Button color="orange" variant="light" onClick={onOverwrite}>
            Save mine anyway
          </Button>
          <Button leftSection={<IconRefresh size={16} />} onClick={onReload} data-autofocus>
            Reload theirs
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

export function RemoteChangeBanner({ onReload }: { onReload(): void }) {
  return (
    <Alert color="orange" icon={<IconAlertTriangle size={18} />} p="xs">
      <Group justify="space-between" wrap="nowrap">
        <Text size="sm">Someone else just saved a change to this transaction.</Text>
        <Button size="xs" variant="light" color="orange" onClick={onReload}>
          Reload
        </Button>
      </Group>
    </Alert>
  );
}

/** QBD's Transaction Journal (Ctrl+Y): the debits and credits this form makes. */
export function PostingsModal({ txn, onClose }: { txn: Txn | null; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const accounts = useAccountIndex();
  const preview = useQuery({
    queryKey: ['preview', txn],
    queryFn: () => api.previewTxn(co, txn!),
    enabled: !!txn,
    retry: false,
  });
  const rows = preview.data?.postings ?? [];
  return (
    <Modal opened={!!txn} onClose={onClose} title="Transaction Journal" size="lg" centered>
      {preview.isLoading && <Loader size="sm" />}
      {preview.error && <Alert color="red">{describeError(preview.error)}</Alert>}
      {preview.data && (
        <Table striped withTableBorder>
          <Table.Thead>
            <Table.Tr>
              <Table.Th>Account</Table.Th>
              <Table.Th>Memo</Table.Th>
              <Table.Th ta="right">Debit</Table.Th>
              <Table.Th ta="right">Credit</Table.Th>
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {rows.map((p, i) => (
              <Table.Tr key={i}>
                <Table.Td>{accounts.get(p.account_id)?.full_name ?? p.account_id}</Table.Td>
                <Table.Td>{p.memo}</Table.Td>
                <Table.Td ta="right">{p.amount > 0 ? formatMoney(p.amount) : ''}</Table.Td>
                <Table.Td ta="right">{p.amount < 0 ? formatMoney(-p.amount) : ''}</Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
          <Table.Tfoot>
            <Table.Tr>
              <Table.Th colSpan={2}>Total</Table.Th>
              <Table.Th ta="right">{formatMoney(rows.reduce((s, p) => s + Math.max(0, p.amount), 0))}</Table.Th>
              <Table.Th ta="right">{formatMoney(rows.reduce((s, p) => s + Math.max(0, -p.amount), 0))}</Table.Th>
            </Table.Tr>
          </Table.Tfoot>
        </Table>
      )}
    </Modal>
  );
}

/** Ctrl+H: who changed this transaction, and when. */
export function HistoryModal({ txnId, onClose }: { txnId: string | null; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const history = useQuery({ queryKey: qk.history(co, txnId ?? ''), queryFn: () => api.history(co, txnId!), enabled: !!txnId });
  return (
    <Modal opened={!!txnId} onClose={onClose} title="Transaction History" size="lg" centered>
      {history.isLoading && <Loader size="sm" />}
      {history.error && <Alert color="red">{describeError(history.error)}</Alert>}
      {history.data && history.data.length === 0 && <Text c="dimmed">No recorded changes yet.</Text>}
      {history.data && history.data.length > 0 && (
        <Table>
          <Table.Tbody>
            {history.data.map((h) => (
              <Table.Tr key={h.id}>
                <Table.Td>{formatDate(h.at.slice(0, 10))}</Table.Td>
                <Table.Td>{h.at.slice(11, 19)}</Table.Td>
                <Table.Td>{h.user ?? '—'}</Table.Td>
                <Table.Td>{h.action}</Table.Td>
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      )}
    </Modal>
  );
}
