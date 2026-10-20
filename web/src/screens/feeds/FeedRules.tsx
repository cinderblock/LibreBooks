// Renaming rules: first match wins, in priority order. Each rule has conditions
// (payee/memo text, amount, direction, account) and what it does (rename, name,
// account, class, memo, add without asking).

import { ActionIcon, Alert, Button, Checkbox, Group, Modal, SegmentedControl, Select, Stack, Switch, Table, Text, TextInput } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconArrowDown, IconArrowUp, IconPencil, IconPlus, IconTrash } from '@tabler/icons-react';
import { useEffect, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex, useAccounts, useCompanyInfo, useEntityIndex, useFeedInvalidate, useFeedRules } from '~/api/hooks';
import type { Condition, FeedRule, TextField, TextOp } from '~/api/types';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, ClassQuickFill, NameQuickFill } from '~/inputs/pickers';
import { formatMoney } from '~/lib/money';
import { useIsMobile } from '~/lib/useMobile';

const blankRule = (): FeedRule => ({
  id: '',
  name: '',
  priority: 0 as unknown as bigint,
  enabled: true,
  match_all: true,
  conditions: [{ kind: 'Text', field: 'Payee', op: 'Contains', value: '' }],
  actions: { rename_to: null, entity_id: null, account_id: null, class_id: null, memo: null, auto_add: false },
  edit_seq: 0 as unknown as bigint,
});

export function FeedRules() {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const rules = useFeedRules();
  const refresh = useFeedInvalidate();
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const [editing, setEditing] = useState<FeedRule | null>(null);
  const list = rules.data ?? [];
  const run = async (f: () => Promise<unknown>, done?: string) => {
    try {
      await f();
      refresh();
      if (done) notifications.show({ message: done, color: 'teal' });
    } catch (e) {
      notifications.show({ color: 'red', message: describeError(e) });
    }
  };
  const move = (i: number, d: -1 | 1) => {
    const ids = list.map((r) => r.id);
    const j = i + d;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j]!, ids[i]!];
    void run(() => api.reorderFeedRules(co, ids));
  };
  const conditionText = (c: Condition) => {
    switch (c.kind) {
      case 'Text':
        return `${c.field === 'PayeeOrMemo' ? 'payee or memo' : c.field.toLowerCase()} ${c.op === 'StartsWith' ? 'starts with' : c.op.toLowerCase()} "${c.value}"`;
      case 'AmountEquals':
        return `amount is ${formatMoney(c.amount)}`;
      case 'AmountRange':
        return `amount ${c.min !== null ? `≥ ${formatMoney(c.min)}` : ''} ${c.max !== null ? `≤ ${formatMoney(c.max)}` : ''}`.trim();
      case 'Direction':
        return c.direction === 'In' ? 'money in' : 'money out';
      case 'Account':
        return `in ${accounts.get(c.account_id)?.full_name ?? 'an account'}`;
    }
  };
  const actionText = (r: FeedRule) => {
    const a = r.actions;
    return [a.rename_to ? `show as "${a.rename_to}"` : null, a.entity_id ? entities.get(a.entity_id)?.full_name : null, a.account_id ? accounts.get(a.account_id)?.full_name : null, a.memo ? `memo "${a.memo}"` : null, a.auto_add ? 'add automatically' : null].filter(Boolean).join(' · ');
  };
  return (
    <Stack gap="sm">
      <Group justify="space-between" wrap="wrap">
        <Text size="sm" c="dimmed">
          The first rule that matches a download decides its suggestion; drag order with the arrows.
        </Text>
        <Group gap="xs">
          <Button size="xs" variant="default" onClick={() => void run(async () => notifications.show({ message: `${await api.applyFeedRules(co)} download(s) got a suggestion.`, color: 'teal' }))}>
            Apply rules to new downloads
          </Button>
          <Button size="xs" leftSection={<IconPlus size={14} />} onClick={() => setEditing(blankRule())}>
            New rule
          </Button>
        </Group>
      </Group>
      {rules.error && <Alert color="red">{describeError(rules.error)}</Alert>}
      <Table fz="sm" withTableBorder aria-label="Rules">
        <Table.Thead>
          <Table.Tr>
            <Table.Th w={60}>On</Table.Th>
            <Table.Th>Rule</Table.Th>
            {!mobile && <Table.Th>When</Table.Th>}
            {!mobile && <Table.Th>Then</Table.Th>}
            <Table.Th w={140} />
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {list.map((r, i) => (
            <Table.Tr key={r.id} data-testid="feed-rule">
              <Table.Td>
                <Switch size="xs" checked={r.enabled} aria-label={`${r.name} enabled`} onChange={(e) => void run(() => api.updateFeedRule(co, { ...r, enabled: e.currentTarget.checked }))} />
              </Table.Td>
              <Table.Td>
                <Text size="sm" fw={500}>
                  {r.name}
                </Text>
                {mobile && (
                  <Text size="xs" c="dimmed">
                    {r.conditions.map(conditionText).join(r.match_all ? ' and ' : ' or ')} → {actionText(r)}
                  </Text>
                )}
              </Table.Td>
              {!mobile && <Table.Td>{r.conditions.map(conditionText).join(r.match_all ? ' and ' : ' or ')}</Table.Td>}
              {!mobile && <Table.Td>{actionText(r)}</Table.Td>}
              <Table.Td>
                <Group gap={2} wrap="nowrap" justify="flex-end">
                  <ActionIcon size="sm" variant="subtle" aria-label="Move up" disabled={i === 0} onClick={() => move(i, -1)}>
                    <IconArrowUp size={14} />
                  </ActionIcon>
                  <ActionIcon size="sm" variant="subtle" aria-label="Move down" disabled={i === list.length - 1} onClick={() => move(i, 1)}>
                    <IconArrowDown size={14} />
                  </ActionIcon>
                  <ActionIcon size="sm" variant="subtle" aria-label={`Edit ${r.name}`} onClick={() => setEditing(r)}>
                    <IconPencil size={14} />
                  </ActionIcon>
                  <ActionIcon size="sm" variant="subtle" color="red" aria-label={`Delete ${r.name}`} onClick={() => window.confirm(`Delete the rule "${r.name}"?`) && void run(() => api.deleteFeedRule(co, r.id), 'Rule deleted.')}>
                    <IconTrash size={14} />
                  </ActionIcon>
                </Group>
              </Table.Td>
            </Table.Tr>
          ))}
        </Table.Tbody>
      </Table>
      {list.length === 0 && !rules.isLoading && <Text c="dimmed" size="sm">No rules yet. Downloads then use how the same payee was added last time.</Text>}
      {editing && <RuleModal rule={editing} onClose={() => setEditing(null)} />}
    </Stack>
  );
}

function RuleModal({ rule, onClose }: { rule: FeedRule; onClose(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const { data: info } = useCompanyInfo();
  const { data: accountList = [] } = useAccounts();
  const [r, setR] = useState<FeedRule>(rule);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => setR(rule), [rule]);
  const setCond = (i: number, c: Condition) => setR({ ...r, conditions: r.conditions.map((x, j) => (j === i ? c : x)) });
  const save = async () => {
    if (!r.name.trim()) return setError('Give the rule a name.');
    if (!r.conditions.length) return setError('Add at least one condition.');
    if (r.actions.auto_add && !r.actions.account_id) return setError('Adding automatically needs an account.');
    setBusy(true);
    try {
      await (r.id ? api.updateFeedRule(co, r) : api.createFeedRule(co, r));
      refresh();
      notifications.show({ message: `Rule "${r.name}" saved.`, color: 'teal' });
      onClose();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  const kindOf = (c: Condition) => c.kind;
  const newCond = (kind: Condition['kind']): Condition => {
    switch (kind) {
      case 'Text':
        return { kind, field: 'Payee', op: 'Contains', value: '' };
      case 'AmountEquals':
        return { kind, amount: 0 };
      case 'AmountRange':
        return { kind, min: null, max: null };
      case 'Direction':
        return { kind, direction: 'Out' };
      case 'Account':
        return { kind, account_id: accountList[0]?.id ?? '' };
    }
  };
  return (
    <Modal opened onClose={onClose} title={r.id ? `Edit rule: ${rule.name}` : 'New rule'} size="lg" centered>
      <Stack gap="sm">
        <TextInput label="Name" value={r.name} onChange={(e) => setR({ ...r, name: e.currentTarget.value })} required data-autofocus />
        <Group gap="xs" align="center">
          <Text size="sm">When</Text>
          <SegmentedControl size="xs" data={[{ value: 'all', label: 'all of these' }, { value: 'any', label: 'any of these' }]} value={r.match_all ? 'all' : 'any'} onChange={(v) => setR({ ...r, match_all: v === 'all' })} />
        </Group>
        {r.conditions.map((c, i) => (
          <Group key={i} gap="xs" align="flex-end" wrap="wrap">
            <Select size="xs" w={140} data={[{ value: 'Text', label: 'Text' }, { value: 'AmountEquals', label: 'Amount is' }, { value: 'AmountRange', label: 'Amount between' }, { value: 'Direction', label: 'Direction' }, { value: 'Account', label: 'Account' }]} value={kindOf(c)} onChange={(v) => v && setCond(i, newCond(v as Condition['kind']))} allowDeselect={false} aria-label="Condition kind" />
            {c.kind === 'Text' && (
              <>
                <Select size="xs" w={130} data={[{ value: 'Payee', label: 'Payee' }, { value: 'Memo', label: 'Memo' }, { value: 'PayeeOrMemo', label: 'Payee or memo' }]} value={c.field} onChange={(v) => v && setCond(i, { ...c, field: v as TextField })} allowDeselect={false} aria-label="Field" />
                <Select size="xs" w={120} data={[{ value: 'Contains', label: 'contains' }, { value: 'StartsWith', label: 'starts with' }, { value: 'Equals', label: 'equals' }, { value: 'Regex', label: 'matches regex' }]} value={c.op} onChange={(v) => v && setCond(i, { ...c, op: v as TextOp })} allowDeselect={false} aria-label="Operator" />
                <TextInput size="xs" value={c.value} onChange={(e) => setCond(i, { ...c, value: e.currentTarget.value })} style={{ flex: 1 }} aria-label="Text to match" />
              </>
            )}
            {c.kind === 'AmountEquals' && <MoneyField size="xs" value={c.amount} onChange={(v) => setCond(i, { ...c, amount: v ?? 0 })} aria-label="Amount" />}
            {c.kind === 'AmountRange' && (
              <>
                <MoneyField size="xs" label="at least" value={c.min} onChange={(v) => setCond(i, { ...c, min: v })} />
                <MoneyField size="xs" label="at most" value={c.max} onChange={(v) => setCond(i, { ...c, max: v })} />
              </>
            )}
            {c.kind === 'Direction' && <SegmentedControl size="xs" data={[{ value: 'Out', label: 'Money out' }, { value: 'In', label: 'Money in' }]} value={c.direction} onChange={(v) => setCond(i, { ...c, direction: v as 'In' | 'Out' })} />}
            {c.kind === 'Account' && <AccountQuickFill size="xs" value={c.account_id || null} onChange={(id) => setCond(i, { ...c, account_id: id ?? '' })} allowAddNew={false} aria-label="Account" style={{ flex: 1 }} />}
            <ActionIcon variant="subtle" color="red" aria-label="Remove condition" onClick={() => setR({ ...r, conditions: r.conditions.filter((_, j) => j !== i) })}>
              <IconTrash size={14} />
            </ActionIcon>
          </Group>
        ))}
        <Button size="xs" variant="subtle" leftSection={<IconPlus size={12} />} onClick={() => setR({ ...r, conditions: [...r.conditions, newCond('Text')] })} w="fit-content">
          Add condition
        </Button>
        <Text size="sm" fw={500}>
          Then
        </Text>
        <Group grow align="flex-end">
          <TextInput size="xs" label="Show the payee as" value={r.actions.rename_to ?? ''} onChange={(e) => setR({ ...r, actions: { ...r.actions, rename_to: e.currentTarget.value || null } })} />
          <NameQuickFill size="xs" label="Name" value={r.actions.entity_id} onChange={(entity_id) => setR({ ...r, actions: { ...r.actions, entity_id } })} aria-label="Name to use" />
        </Group>
        <Group grow align="flex-end">
          <AccountQuickFill size="xs" label="Account" value={r.actions.account_id} onChange={(account_id) => setR({ ...r, actions: { ...r.actions, account_id } })} aria-label="Account to use" />
          {info?.preferences.use_class_tracking ? <ClassQuickFill size="xs" label="Class" value={r.actions.class_id} onChange={(class_id) => setR({ ...r, actions: { ...r.actions, class_id } })} /> : <TextInput size="xs" label="Memo" value={r.actions.memo ?? ''} onChange={(e) => setR({ ...r, actions: { ...r.actions, memo: e.currentTarget.value || null } })} />}
        </Group>
        <Checkbox size="xs" label="Add to the register without asking when nothing in it matches the download" checked={r.actions.auto_add} onChange={(e) => setR({ ...r, actions: { ...r.actions, auto_add: e.currentTarget.checked } })} />
        {error && <Alert color="red">{error}</Alert>}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button loading={busy} onClick={() => void save()}>
            Save
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
