// The downloaded transactions: filter by account and status, then Match (pick
// a register line from the candidates), Add (an inline row with the name,
// account, class and memo the rules suggest, with a preview of the postings),
// "More…" (the same, prefilled, in the full form), Ignore, and Undo.

import { Alert, Badge, Button, Card, Checkbox, Group, Modal, Progress, SegmentedControl, Select, Stack, Table, Text, TextInput } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconSearch } from '@tabler/icons-react';
import { useMemo, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex, useCompanyInfo, useEntityIndex, useFeedCandidates, useFeedInvalidate, useFeedTxns, useFeedsSummary } from '~/api/hooks';
import type { AddRequest, FeedStatus, FeedTxn, MatchCandidate } from '~/api/types';
import { formSlugFor } from '~/forms/registry';
import { PostingsPanel } from '~/forms/SalesForm';
import { AccountQuickFill, ClassQuickFill, NameQuickFill } from '~/inputs/pickers';
import { formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { stashTxn } from '~/windows/handoff';
import { useOpenWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from '../screens.module.css';

const STATUS_COLOR: Record<FeedStatus, string> = { New: 'blue', Matched: 'teal', Added: 'green', Ignored: 'gray' };

export function FeedQueue({ accountId, onAccountChange }: { accountId: string | null; onAccountChange(id: string | null): void }) {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const summary = useFeedsSummary();
  const refresh = useFeedInvalidate();
  const open = useOpenWindow();
  const [status, setStatus] = useState<'New' | 'Matched' | 'Added' | 'Ignored' | 'all'>('New');
  const [text, setText] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [matching, setMatching] = useState<FeedTxn | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const list = useFeedTxns({ account_id: accountId, statuses: status === 'all' ? [] : [status], text: text || null, descending: true, limit: 500 });
  const rows = list.data ?? [];
  const accountOptions = useMemo(() => (summary.data ?? []).map((s) => ({ value: s.account_id, label: accounts.get(s.account_id)?.full_name ?? s.account_id })), [summary.data, accounts]);

  const run = async (label: string, f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
      refresh();
      if (label) notifications.show({ message: label, color: 'teal' });
    } catch (e) {
      notifications.show({ color: 'red', message: describeError(e) });
    } finally {
      setBusy(false);
    }
  };
  const ignore = (ids: string[]) => run(`${ids.length} ignored.`, () => api.feedIgnore(co, ids)).then(() => setSelected(new Set()));
  const addAll = (ids: string[]) =>
    run('', async () => {
      const out = await api.feedAddAll(co, ids);
      notifications.show({ message: `${out.added.length} added${out.failed.length ? `, ${out.failed.length} failed: ${out.failed.map((f) => f.message).join('; ')}` : '.'}`, color: out.failed.length ? 'orange' : 'teal' });
    }).then(() => setSelected(new Set()));
  const undo = (t: FeedTxn) => run('Undone.', () => api.feedUndo(co, t.id));
  /** "More…": the transaction Add would write, opened in its form to finish by hand. */
  const more = async (t: FeedTxn) => {
    try {
      const p = await api.feedPreview(co, t.id, { entity_id: null, account_id: null, class_id: null, memo: null, number: null });
      const slug = formSlugFor(p.txn.txn_type);
      if (!slug) throw new Error(`No form for ${p.txn.txn_type}`);
      open(slug === 'check' ? paths.newCheck(stashTxn(p.txn)) : paths.form(slug, stashTxn(p.txn)));
      notifications.show({ message: 'Save the form, then match this download to it.', color: 'blue' });
    } catch (e) {
      notifications.show({ color: 'red', message: describeError(e) });
    }
  };

  const suggestionText = (t: FeedTxn) => {
    const s = t.suggestion;
    if (!s) return null;
    const parts = [s.entity_id ? entities.get(s.entity_id)?.full_name : null, s.account_id ? accounts.get(s.account_id)?.full_name : null].filter(Boolean);
    return `${parts.join(' · ')}${s.rule_id ? ' (rule)' : s.from_history ? ' (last time)' : ''}${s.auto_add ? ' · auto-add' : ''}`;
  };

  const newIds = rows.filter((r) => r.status === 'New').map((r) => r.id);
  const selectedNew = [...selected].filter((id) => newIds.includes(id));

  const actions = (t: FeedTxn) =>
    t.status === 'New' ? (
      <Group gap={4} wrap="nowrap" justify="flex-end">
        <Button size="compact-xs" variant="light" onClick={() => setMatching(t)}>
          Match
        </Button>
        <Button size="compact-xs" variant="light" color="green" onClick={() => setAdding(adding === t.id ? null : t.id)}>
          Add
        </Button>
        <Button size="compact-xs" variant="subtle" onClick={() => void more(t)}>
          More…
        </Button>
        <Button size="compact-xs" variant="subtle" color="gray" onClick={() => void ignore([t.id])}>
          Ignore
        </Button>
      </Group>
    ) : (
      <Group gap={4} wrap="nowrap" justify="flex-end">
        {t.matched_txn_id && (
          <Button size="compact-xs" variant="subtle" onClick={() => open(paths.txn(t.matched_txn_id!))}>
            Open
          </Button>
        )}
        <Button size="compact-xs" variant="subtle" color="gray" onClick={() => void undo(t)}>
          Undo
        </Button>
      </Group>
    );

  return (
    <Stack gap="sm" style={{ flex: 1, minHeight: 0 }}>
      <Group gap="xs" wrap="wrap">
        <Select size="xs" placeholder="All accounts" data={accountOptions} value={accountId} onChange={onAccountChange} clearable w={mobile ? '100%' : 240} aria-label="Account" />
        <SegmentedControl
          size="xs"
          data={[
            { value: 'New', label: 'New' },
            { value: 'Matched', label: 'Matched' },
            { value: 'Added', label: 'Added' },
            { value: 'Ignored', label: 'Ignored' },
            { value: 'all', label: 'All' },
          ]}
          value={status}
          onChange={(v) => setStatus(v as typeof status)}
        />
        <TextInput size="xs" placeholder="Find" leftSection={<IconSearch size={12} />} value={text} onChange={(e) => setText(e.currentTarget.value)} w={mobile ? '100%' : 180} aria-label="Find downloads" />
        {selectedNew.length > 0 && (
          <Group gap={4} ml="auto">
            <Button size="xs" color="green" loading={busy} onClick={() => void addAll(selectedNew)}>
              Add {selectedNew.length}
            </Button>
            <Button size="xs" variant="default" loading={busy} onClick={() => void ignore(selectedNew)}>
              Ignore {selectedNew.length}
            </Button>
          </Group>
        )}
      </Group>
      {list.error && <Alert color="red">{describeError(list.error)}</Alert>}
      {rows.length === 0 && !list.isLoading && (
        <Text c="dimmed" size="sm">
          Nothing {status === 'all' ? 'downloaded' : status.toLowerCase()} here.
        </Text>
      )}
      {mobile ? (
        <Stack gap="xs" style={{ overflow: 'auto', flex: 1, minHeight: 0 }}>
          {rows.map((t) => (
            <Card key={t.id} withBorder padding="sm" data-testid="feed-row" style={{ flexShrink: 0 }}>
              <Group justify="space-between" wrap="nowrap">
                <div style={{ minWidth: 0 }}>
                  <Text size="sm" fw={600} truncate>
                    {t.suggestion?.payee ?? t.payee}
                  </Text>
                  <Text size="xs" c="dimmed" truncate>
                    {formatDateShort(t.posted)} · {t.memo ?? ''} {suggestionText(t) ? `→ ${suggestionText(t)}` : ''}
                  </Text>
                </div>
                <Text size="sm" fw={600} className={classes.num} c={t.amount < 0 ? undefined : 'teal'}>
                  {formatMoney(t.amount)}
                </Text>
              </Group>
              <Group justify="space-between" mt="xs">
                <Badge size="xs" color={STATUS_COLOR[t.status]}>
                  {t.status}
                </Badge>
                {actions(t)}
              </Group>
              {adding === t.id && <AddRow txn={t} onDone={() => setAdding(null)} />}
            </Card>
          ))}
        </Stack>
      ) : (
        <div style={{ overflow: 'auto', flex: 1, minHeight: 0 }}>
          <Table fz="sm" withTableBorder highlightOnHover stickyHeader aria-label="Downloaded transactions">
            <Table.Thead>
              <Table.Tr>
                <Table.Th w={32}>
                  <Checkbox size="xs" aria-label="Select all new" checked={newIds.length > 0 && newIds.every((id) => selected.has(id))} onChange={(e) => setSelected(e.currentTarget.checked ? new Set(newIds) : new Set())} />
                </Table.Th>
                <Table.Th>Date</Table.Th>
                <Table.Th>Downloaded as</Table.Th>
                <Table.Th>Suggestion</Table.Th>
                <Table.Th ta="right">Amount</Table.Th>
                <Table.Th>Status</Table.Th>
                <Table.Th />
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {rows.map((t) => (
                <>
                  <Table.Tr key={t.id} data-testid="feed-row" data-status={t.status}>
                    <Table.Td>{t.status === 'New' && <Checkbox size="xs" aria-label={`Select ${t.payee ?? ''}`} checked={selected.has(t.id)} onChange={(e) => setSelected((s) => { const n = new Set(s); if (e.currentTarget.checked) n.add(t.id); else n.delete(t.id); return n; })} />}</Table.Td>
                    <Table.Td>{formatDateShort(t.posted)}</Table.Td>
                    <Table.Td>
                      <Text size="sm">{t.payee}</Text>
                      {(t.memo || t.check_number) && (
                        <Text size="xs" c="dimmed">
                          {t.check_number ? `#${t.check_number} ` : ''}
                          {t.memo}
                        </Text>
                      )}
                    </Table.Td>
                    <Table.Td>
                      <Text size="xs" c="dimmed">
                        {t.suggestion?.payee ? `${t.suggestion.payee} · ` : ''}
                        {suggestionText(t) ?? '—'}
                      </Text>
                    </Table.Td>
                    <Table.Td ta="right" className={classes.num} c={t.amount < 0 ? undefined : 'teal'}>
                      {formatMoney(t.amount)}
                    </Table.Td>
                    <Table.Td>
                      <Badge size="xs" color={STATUS_COLOR[t.status]}>
                        {t.status}
                      </Badge>
                    </Table.Td>
                    <Table.Td>{actions(t)}</Table.Td>
                  </Table.Tr>
                  {adding === t.id && (
                    <Table.Tr key={`${t.id}-add`}>
                      <Table.Td colSpan={7}>
                        <AddRow txn={t} onDone={() => setAdding(null)} />
                      </Table.Td>
                    </Table.Tr>
                  )}
                </>
              ))}
            </Table.Tbody>
          </Table>
        </div>
      )}
      <CandidatesModal txn={matching} onClose={() => setMatching(null)} onAdd={(t) => { setMatching(null); setAdding(t.id); }} />
    </Stack>
  );
}

/** The inline "Add" editor: what the rules suggest, editable, with the postings it would make. */
function AddRow({ txn, onDone }: { txn: FeedTxn; onDone(): void }) {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const accounts = useAccountIndex();
  const { data: info } = useCompanyInfo();
  const s = txn.suggestion;
  const [req, setReq] = useState<AddRequest>({ entity_id: s?.entity_id ?? null, account_id: s?.account_id ?? null, class_id: s?.class_id ?? null, memo: s?.memo ?? txn.memo ?? null, number: txn.check_number });
  const [preview, setPreview] = useState<{ postings: Array<{ account_id: string; amount: number; memo: string | null }>; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const isCheck = txn.amount < 0 && accounts.get(txn.account_id)?.account_type === 'Bank';
  const doPreview = async () => {
    try {
      const p = await api.feedPreview(co, txn.id, req);
      setPreview({ postings: p.postings, total: p.total });
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  };
  const add = async () => {
    setBusy(true);
    try {
      const out = await api.feedAdd(co, txn.id, req);
      refresh();
      notifications.show({ message: `Added as ${TXN_LABEL[out.txn_type]}.`, color: 'teal' });
      onDone();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Stack gap="xs" p="xs" data-testid="feed-add-row">
      <Group gap="sm" align="flex-end" wrap="wrap">
        <NameQuickFill label="Name" size="xs" value={req.entity_id} onChange={(entity_id) => setReq({ ...req, entity_id })} aria-label="Name" style={{ flex: '1 1 200px' }} />
        <AccountQuickFill label="Account" size="xs" value={req.account_id} onChange={(account_id) => setReq({ ...req, account_id })} aria-label="Account" style={{ flex: '1 1 220px' }} />
        {info?.preferences.use_class_tracking && <ClassQuickFill label="Class" size="xs" value={req.class_id} onChange={(class_id) => setReq({ ...req, class_id })} aria-label="Class" />}
        <TextInput label="Memo" size="xs" value={req.memo ?? ''} onChange={(e) => setReq({ ...req, memo: e.currentTarget.value || null })} aria-label="Memo" style={{ flex: '1 1 160px' }} />
        {isCheck && <TextInput label="Check no." size="xs" value={req.number ?? ''} onChange={(e) => setReq({ ...req, number: e.currentTarget.value || null })} w={90} aria-label="Check number" />}
        <Button size="xs" variant="default" onClick={() => void doPreview()}>
          Preview
        </Button>
        <Button size="xs" color="green" loading={busy} onClick={() => void add()} disabled={!req.account_id}>
          Add to register
        </Button>
        <Button size="xs" variant="subtle" onClick={onDone}>
          Cancel
        </Button>
      </Group>
      {error && (
        <Text size="xs" c="red">
          {error}
        </Text>
      )}
      {preview && <PostingsPanel postings={preview.postings} accounts={accounts} error={null} />}
    </Stack>
  );
}

function CandidatesModal({ txn, onClose, onAdd }: { txn: FeedTxn | null; onClose(): void; onAdd(t: FeedTxn): void }) {
  const api = useApi();
  const co = useCompanyId();
  const refresh = useFeedInvalidate();
  const candidates = useFeedCandidates(txn?.id ?? null);
  const open = useOpenWindow();
  const [busy, setBusy] = useState(false);
  const match = async (c: MatchCandidate) => {
    if (!txn) return;
    setBusy(true);
    try {
      await api.feedMatch(co, txn.id, c.txn_id);
      refresh();
      notifications.show({ message: `Matched to ${TXN_LABEL[c.txn_type]} ${c.number ?? ''}.`, color: 'teal' });
      onClose();
    } catch (e) {
      notifications.show({ color: 'red', message: describeError(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal opened={!!txn} onClose={onClose} title="Match to a transaction in the register" size="lg" centered>
      {txn && (
        <Stack gap="sm">
          <Text size="sm">
            {formatDateShort(txn.posted)} · {txn.payee} · <b>{formatMoney(txn.amount)}</b>
          </Text>
          {candidates.isLoading && <Text size="sm" c="dimmed">Looking…</Text>}
          {candidates.data?.length === 0 && (
            <Alert color="gray">
              Nothing in the register has this amount within the match window. Add it, or enter it by hand and match it afterwards.
            </Alert>
          )}
          {(candidates.data ?? []).map((c) => (
            <Card key={c.txn_id} withBorder padding="sm" data-strong={c.strong || undefined} data-testid="feed-candidate">
              <Group justify="space-between" wrap="nowrap" align="flex-start">
                <div style={{ minWidth: 0, flex: 1 }}>
                  <Group gap="xs">
                    <Text size="sm" fw={600}>
                      {TXN_LABEL[c.txn_type]} {c.number ?? ''}
                    </Text>
                    <Text size="sm">{formatDateShort(c.date)}</Text>
                    <Text size="sm">{c.names ?? c.memo}</Text>
                    {c.strong && (
                      <Badge size="xs" color="teal">
                        Likely
                      </Badge>
                    )}
                  </Group>
                  <Progress value={c.score} size="xs" mt={4} color={c.strong ? 'teal' : 'gray'} aria-label={`Score ${c.score}`} />
                  <Text size="xs" c="dimmed" mt={2}>
                    {c.reasons.join(' · ')}
                  </Text>
                </div>
                <Group gap={4} wrap="nowrap">
                  <Button size="compact-xs" variant="subtle" onClick={() => open(paths.txn(c.txn_id))}>
                    Open
                  </Button>
                  <Button size="compact-xs" loading={busy} onClick={() => void match(c)}>
                    Match
                  </Button>
                </Group>
              </Group>
            </Card>
          ))}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Close
            </Button>
            <Button variant="light" color="green" onClick={() => onAdd(txn)}>
              Add instead
            </Button>
          </Group>
        </Stack>
      )}
    </Modal>
  );
}
