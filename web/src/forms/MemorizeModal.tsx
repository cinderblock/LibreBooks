// Ctrl+M on a form: QBD's Memorize Transaction dialog. The form as it stands
// becomes the template; the schedule says whether LibreBooks only keeps it
// ("Never"), lists it when due ("Remind Me"), or enters it by itself
// ("Automatically Enter"). Saved as a `memorized_txn` document.

import { Alert, Button, Group, Modal, NumberInput, SegmentedControl, Select, Stack, Text, TextInput } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { useEffect, useState } from 'react';
import { describeError } from '~/api/errors';
import { useEntityIndex, useSaveSaved } from '~/api/hooks';
import type { Interval, Memorized, Repeat, Txn } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { addMonths, todayIso } from '~/lib/dates';
import { TXN_LABEL } from '~/lib/txnTypes';

export const INTERVAL_LABEL: Record<Interval, string> = {
  Daily: 'Daily',
  Weekly: 'Weekly',
  BiWeekly: 'Every two weeks',
  SemiMonthly: 'Twice a month',
  Monthly: 'Monthly',
  Quarterly: 'Quarterly',
  SemiAnnually: 'Every six months',
  Annually: 'Annually',
};

export const REPEAT_LABEL: Record<Repeat, string> = { Never: 'Do not remind me', Remind: 'Remind me', Automatic: 'Automatically enter' };

export function MemorizeModal({ txn, onClose, existing }: { txn: Txn | null; onClose(): void; existing?: { id: string; name: string; memorized: Memorized } | null }) {
  const entities = useEntityIndex();
  const save = useSaveSaved('memorized_txn');
  const [name, setName] = useState('');
  const [repeat, setRepeat] = useState<Repeat>('Remind');
  const [interval, setInterval] = useState<Interval>('Monthly');
  const [nextDate, setNextDate] = useState<string | null>(null);
  const [daysInAdvance, setDaysInAdvance] = useState(0);
  const [remaining, setRemaining] = useState<number | null>(null);
  const [group, setGroup] = useState('');

  useEffect(() => {
    if (!txn) return;
    const m = existing?.memorized;
    setName(existing?.name ?? (txn.entity_id ? (entities.get(txn.entity_id)?.full_name ?? '') : TXN_LABEL[txn.txn_type]));
    setRepeat(m?.repeat ?? 'Remind');
    setInterval(m?.interval ?? 'Monthly');
    setNextDate(m?.next_date ?? addMonths(txn.date || todayIso(), 1));
    setDaysInAdvance(m?.days_in_advance ?? 0);
    setRemaining(m?.remaining ?? null);
    setGroup(m?.group ?? '');
  }, [txn, existing]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!txn) return null;
  const scheduled = repeat !== 'Never';
  const submit = () => {
    const doc: Memorized = {
      // The template keeps the form as it stands; the server replaces id, number and edit_seq on use.
      template: { ...txn, edit_seq: 0, is_void: false },
      repeat,
      interval: scheduled ? interval : null,
      next_date: scheduled ? nextDate : null,
      days_in_advance: scheduled ? daysInAdvance : 0,
      remaining: scheduled ? remaining : null,
      group: group.trim() || null,
    };
    save.mutate(
      { id: existing?.id ?? null, name: name.trim(), doc },
      {
        onSuccess: () => {
          notifications.show({ message: `Memorized "${name.trim()}".`, color: 'teal' });
          onClose();
        },
      },
    );
  };
  return (
    <Modal opened onClose={onClose} title="Memorize Transaction" centered>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <Stack gap="sm">
          <TextInput label="Name" value={name} onChange={(e) => setName(e.currentTarget.value)} required data-autofocus />
          <SegmentedControl fullWidth data={(Object.keys(REPEAT_LABEL) as Repeat[]).map((r) => ({ value: r, label: REPEAT_LABEL[r] }))} value={repeat} onChange={(v) => setRepeat(v as Repeat)} />
          {scheduled && (
            <>
              <Group grow>
                <Select label="How often" data={(Object.keys(INTERVAL_LABEL) as Interval[]).map((i) => ({ value: i, label: INTERVAL_LABEL[i] }))} value={interval} onChange={(v) => v && setInterval(v as Interval)} allowDeselect={false} />
                <DateField label="Next date" value={nextDate} onChange={setNextDate} />
              </Group>
              <Group grow>
                <NumberInput label="Days in advance to enter" value={daysInAdvance} onChange={(v) => setDaysInAdvance(Number(v) || 0)} min={0} />
                <NumberInput label="Number remaining" placeholder="forever" value={remaining ?? ''} onChange={(v) => setRemaining(v === '' ? null : Number(v))} min={0} />
              </Group>
            </>
          )}
          <TextInput label="Group" placeholder="(none)" value={group} onChange={(e) => setGroup(e.currentTarget.value)} />
          <Text size="xs" c="dimmed">
            {repeat === 'Automatic'
              ? 'LibreBooks enters it on its own when due, on the server.'
              : repeat === 'Remind'
                ? 'It appears in the due list on the Memorized Transaction List (Ctrl+T) when its date comes.'
                : 'It waits on the Memorized Transaction List (Ctrl+T) until you enter it.'}
          </Text>
          {save.error && <Alert color="red">{describeError(save.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" loading={save.isPending} disabled={!name.trim()}>
              {existing ? 'Save' : 'Memorize'}
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}
