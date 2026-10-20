// Merge (Company → Merge…; admin): QBD merges two names, accounts or items by
// renaming one to the other and answering "Yes" to the prompt. Here it's an
// explicit action: pick what disappears and what it folds into; every
// transaction that pointed at the first points at the second afterwards.

import { Alert, Button, Card, Group, SegmentedControl, Stack, Text, Title } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconArrowsJoin } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex, useEntityIndex, useInvalidate, useItemIndex } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { MergeKind } from '~/api/types';
import { AccountQuickFill, ItemQuickFill, NameQuickFill } from '~/inputs/pickers';
import { useIsMobile } from '~/lib/useMobile';
import { useWindow } from '~/windows/WindowsProvider';
import { AdminGate, Confirm } from './admin/common';
import classes from './screens.module.css';

const KIND_LABEL: Record<MergeKind, string> = { entity: 'Names', account: 'Accounts', item: 'Items' };

export function MergeWindow() {
  const win = useWindow();
  useEffect(() => win.setTitle('Merge'), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <AdminGate what="Merging records">
      <Merge />
    </AdminGate>
  );
}

function Merge() {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const invalidate = useInvalidate();
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const items = useItemIndex();
  const [kind, setKind] = useState<MergeKind>('entity');
  const [from, setFrom] = useState<string | null>(null);
  const [into, setInto] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const nameOf = (id: string | null) => (id ? ((kind === 'account' ? accounts.get(id)?.full_name : kind === 'entity' ? entities.get(id)?.full_name : items.get(id)?.full_name) ?? id) : '');
  const merge = useMutation({
    mutationFn: () => api.merge(co, { kind, from: from!, into: into! }),
    onSuccess: () => {
      invalidate(qk.company(co));
      notifications.show({ message: `Merged "${nameOf(from)}" into "${nameOf(into)}".`, color: 'teal' });
      setConfirming(false);
      setFrom(null);
      setInto(null);
    },
    onError: () => setConfirming(false),
  });
  const Picker = kind === 'account' ? AccountQuickFill : kind === 'entity' ? NameQuickFill : ItemQuickFill;
  const same = !!from && from === into;
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} maw={640} data-testid="merge">
      <div>
        <Title order={3}>Merge</Title>
        <Text size="sm" c="dimmed">
          Fold a duplicate into the record you want to keep. Every transaction, subaccount and default that pointed at the first points at the second afterwards, and the first is gone. There is no undo.
        </Text>
      </div>
      <SegmentedControl fullWidth data={(Object.keys(KIND_LABEL) as MergeKind[]).map((k) => ({ value: k, label: KIND_LABEL[k] }))} value={kind} onChange={(v) => { setKind(v as MergeKind); setFrom(null); setInto(null); }} />
      <Card withBorder>
        <Stack gap="sm">
          <Picker key={`from-${kind}`} label="Merge this (it disappears)" value={from} onChange={setFrom} allowAddNew={false} showInactive aria-label="Record to merge away" />
          <Group justify="center">
            <IconArrowsJoin size={20} style={{ transform: 'rotate(90deg)' }} />
          </Group>
          <Picker key={`into-${kind}`} label="Into this (it stays, with its own name)" value={into} onChange={setInto} allowAddNew={false} showInactive aria-label="Record to keep" error={same ? 'Choose two different records' : undefined} />
        </Stack>
      </Card>
      {kind === 'account' && (
        <Text size="xs" c="dimmed">
          Accounts must be the same type. Names must be the same kind (two vendors, two customers). Items must be the same item type.
        </Text>
      )}
      {merge.error && <Alert color="red">{describeError(merge.error)}</Alert>}
      <Group justify="flex-end">
        <Button disabled={!from || !into || same} onClick={() => setConfirming(true)}>
          Merge…
        </Button>
      </Group>
      <Confirm opened={confirming} title="Merge these two?" confirmLabel="Merge" loading={merge.isPending} onConfirm={() => merge.mutate()} onClose={() => setConfirming(false)}>
        <Text size="sm">
          <b>{nameOf(from)}</b> will be merged into <b>{nameOf(into)}</b>. Its history moves over; its name is gone. This can't be undone.
        </Text>
      </Confirm>
    </Stack>
  );
}
