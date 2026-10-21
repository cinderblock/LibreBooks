// Attachments (Company → Attachments, and the paperclip on a form): receipts
// and documents. Upload a file or take a photo (`<input capture>` opens the
// camera on a phone); what isn't attached to anything yet waits in the inbox
// until it's attached to a record: a transaction, a name, an account or an
// item (every `record_kind` the API takes). The Attached tab lists every
// document that has found its record (`GET /attachments?all=true`) with a way
// into it. Opened from a form or an editor (the paperclip), the window shows
// that record's files first and offers to attach inbox items to it.

import { ActionIcon, Alert, Anchor, Badge, Button, Card, FileButton, Group, Image, Modal, SegmentedControl, SimpleGrid, Stack, Table, Tabs, Text, TextInput, Title, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconCamera, IconExternalLink, IconPaperclip, IconTrash, IconUpload } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAccountIndex, useAttachments, useEntityIndex, useInvalidate, useItemIndex, useTxns } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { AttachmentMeta, AttachmentRecord } from '~/api/types';
import { AccountQuickFill, ItemQuickFill, NameQuickFill } from '~/inputs/pickers';
import { formatDateShort } from '~/lib/dates';
import { formatBytes } from '~/lib/format';
import { formatMoney } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { useOpenWindow, useWindow } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { Confirm, When } from './admin/common';
import classes from './screens.module.css';

export function Attachments() {
  const win = useWindow();
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const invalidate = useInvalidate();
  const kind = win.desc.params.kind;
  const id = win.desc.params.id;
  const record: AttachmentRecord = kind && id ? { record_kind: kind, record_id: id } : null;
  const recordName = useRecordName(record);
  const noun = record ? (RECORD_NOUN[record.record_kind] ?? 'record') : 'record';
  const [tab, setTab] = useState<'inbox' | 'attached'>('inbox');
  const inbox = useAttachments({});
  const mine = useAttachments(record ? { record_kind: record.record_kind, record_id: record.record_id } : {}, !!record);
  // Every document, fetched only while the Attached tab shows; the inbox rows are already on their own tab.
  const everything = useAttachments({ all: true }, tab === 'attached');
  const attached = useMemo(() => (everything.data ?? []).filter((a) => a.record_kind && a.record_id), [everything.data]);
  const [attaching, setAttaching] = useState<AttachmentMeta | null>(null);
  const [deleting, setDeleting] = useState<AttachmentMeta | null>(null);
  const camera = useRef<HTMLInputElement>(null);
  useEffect(() => win.setTitle(record ? `Attachments: ${recordName ?? `this ${noun}`}` : 'Attachments'), [record?.record_id, recordName]); // eslint-disable-line react-hooks/exhaustive-deps

  const refresh = () => invalidate(qk.attachmentsAll(co));
  const upload = useMutation({
    mutationFn: async (files: File[]) => {
      const out: AttachmentMeta[] = [];
      for (const f of files) out.push(await api.uploadAttachment(co, f, record));
      return out;
    },
    onSuccess: (metas) => {
      refresh();
      notifications.show({ message: metas.length === 1 ? `Added ${metas[0]!.filename}${record ? ` to this ${noun}` : ' to the inbox'}.` : `Added ${metas.length} files.`, color: 'teal' });
    },
  });
  const attach = useMutation({
    mutationFn: ({ a, to }: { a: AttachmentMeta; to: AttachmentRecord }) => api.attachRecord(co, a.id, to),
    onSuccess: (_m, { to }) => {
      refresh();
      setAttaching(null);
      notifications.show({ message: to ? 'Attached.' : 'Moved back to the inbox.', color: 'teal' });
    },
  });
  const remove = useMutation({
    mutationFn: (a: AttachmentMeta) => api.deleteAttachment(co, a.id),
    onSuccess: () => {
      refresh();
      setDeleting(null);
    },
  });

  const pick = (files: File | File[] | null) => {
    const list = files === null ? [] : Array.isArray(files) ? files : [files];
    if (list.length) upload.mutate(list);
  };

  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="md" className={classes.page} data-testid="attachments">
      <Group justify="space-between" align="flex-start" wrap="wrap">
        <div>
          <Title order={3}>Attachments</Title>
          <Text size="sm" c="dimmed">
            {record ? `Files on ${recordName ? `${recordName} (${noun})` : `this ${noun}`}, and the inbox to attach from.` : 'Receipts and documents. Files land in the inbox until they are attached to a transaction, a name, an account or an item.'}
          </Text>
        </div>
        <Group gap="xs">
          {/* The camera input: `capture` asks a phone for its camera rather than the gallery. */}
          <input ref={camera} type="file" accept="image/*" capture="environment" hidden aria-label="Take a photo" onChange={(e) => pick(e.currentTarget.files ? [...e.currentTarget.files] : null)} />
          <Button variant="default" leftSection={<IconCamera size={16} />} onClick={() => camera.current?.click()} loading={upload.isPending}>
            Take a photo
          </Button>
          <FileButton onChange={pick} multiple accept="image/*,application/pdf,.csv,.txt,.doc,.docx,.xls,.xlsx">
            {(props) => (
              <Button {...props} leftSection={<IconUpload size={16} />} loading={upload.isPending}>
                Add files
              </Button>
            )}
          </FileButton>
        </Group>
      </Group>
      {upload.error && <Alert color="red">{describeError(upload.error)}</Alert>}
      {attach.error && !attaching && <Alert color="red">{describeError(attach.error)}</Alert>}

      {record && (
        <Card withBorder>
          <Text fw={600} mb="xs">
            On this {noun}
          </Text>
          <AttachmentTable rows={mine.data ?? []} loading={mine.isLoading} empty="Nothing attached yet. Add a file above, or attach one from the inbox below." onDetach={(a) => attach.mutate({ a, to: null })} onDelete={setDeleting} />
        </Card>
      )}

      <Card withBorder>
        <Tabs value={tab} onChange={(v) => setTab(v === 'attached' ? 'attached' : 'inbox')} keepMounted={false}>
          <Tabs.List mb="xs">
            <Tabs.Tab value="inbox" rightSection={(inbox.data?.length ?? 0) > 0 ? <Badge variant="light" size="sm">{inbox.data!.length}</Badge> : undefined}>
              Inbox
            </Tabs.Tab>
            <Tabs.Tab value="attached">Attached</Tabs.Tab>
          </Tabs.List>
          <Tabs.Panel value="inbox">
            <AttachmentTable
              rows={inbox.data ?? []}
              loading={inbox.isLoading}
              empty="The inbox is empty."
              onAttach={record ? (a) => attach.mutate({ a, to: record }) : setAttaching}
              attachLabel={record ? 'Attach here' : 'Attach to…'}
              onDelete={setDeleting}
            />
          </Tabs.Panel>
          <Tabs.Panel value="attached">
            {everything.error ? (
              <Alert color="red">{describeError(everything.error)}</Alert>
            ) : (
              <AttachmentTable rows={attached} loading={everything.isLoading} empty="Nothing is attached to a record yet." showRecord onDetach={(a) => attach.mutate({ a, to: null })} onDelete={setDeleting} />
            )}
          </Tabs.Panel>
        </Tabs>
      </Card>

      <AttachToModal attachment={attaching} onClose={() => setAttaching(null)} onPick={(to) => attaching && attach.mutate({ a: attaching, to })} busy={attach.isPending} error={attach.error} />
      <Confirm opened={!!deleting} title={`Delete "${deleting?.filename}"?`} confirmLabel="Delete" loading={remove.isPending} onConfirm={() => deleting && remove.mutate(deleting)} onClose={() => setDeleting(null)}>
        <Text size="sm">The file is removed from the books for good.</Text>
        {remove.error && <Alert color="red" mt="sm">{describeError(remove.error)}</Alert>}
      </Confirm>
    </Stack>
  );
}

/** What a document is attached to, for the Attached tab: a button into the transaction, or the name's / account's / item's editor. */
function RecordLink({ a }: { a: AttachmentMeta }) {
  const open = useOpenWindow();
  const name = useRecordName(a.record_kind && a.record_id ? { record_kind: a.record_kind, record_id: a.record_id } : null);
  if (!a.record_kind || !a.record_id) return null;
  const id = a.record_id;
  const path = a.record_kind === 'txn' ? paths.txn(id) : a.record_kind === 'entity' ? paths.record('entities', id) : a.record_kind === 'account' ? paths.record('accounts', id) : a.record_kind === 'item' ? paths.record('items', id) : null;
  if (!path) {
    return (
      <Text size="xs" c="dimmed">
        Attached to a {RECORD_NOUN[a.record_kind] ?? a.record_kind}
      </Text>
    );
  }
  return (
    <Button size="compact-xs" variant="subtle" rightSection={<IconExternalLink size={12} />} onClick={() => open(path)} title={name ?? undefined}>
      Open {RECORD_NOUN[a.record_kind] ?? a.record_kind}
      {name ? `: ${name}` : ''}
    </Button>
  );
}

/** The API's `record_kind`s, as people call them. */
const RECORD_NOUN: Record<string, string> = { txn: 'transaction', entity: 'name', account: 'account', item: 'item' };

/** A record's name from the lists (transactions are named in the Attach dialog's rows instead). */
function useRecordName(record: AttachmentRecord): string | null {
  const entities = useEntityIndex();
  const accounts = useAccountIndex();
  const items = useItemIndex();
  if (!record) return null;
  const id = record.record_id;
  switch (record.record_kind) {
    case 'entity':
      return entities.get(id)?.full_name ?? null;
    case 'account':
      return accounts.get(id)?.full_name ?? null;
    case 'item':
      return items.get(id)?.full_name ?? null;
    default:
      return null;
  }
}

function AttachmentTable({ rows, loading, empty, onAttach, attachLabel, onDetach, onDelete, showRecord }: { rows: AttachmentMeta[]; loading: boolean; empty: string; onAttach?(a: AttachmentMeta): void; attachLabel?: string; onDetach?(a: AttachmentMeta): void; onDelete(a: AttachmentMeta): void; showRecord?: boolean }) {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  if (!loading && rows.length === 0) {
    return (
      <Text size="sm" c="dimmed">
        {empty}
      </Text>
    );
  }
  if (mobile) {
    return (
      <SimpleGrid cols={1} spacing="xs">
        {rows.map((a) => (
          <Card key={a.id} withBorder p="xs" data-testid="attachment-row">
            <Group wrap="nowrap" align="flex-start">
              <Thumb a={a} url={api.attachmentUrl(co, a.id)} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <Anchor href={api.attachmentUrl(co, a.id)} target="_blank" rel="noreferrer" size="sm" lineClamp={1}>
                  {a.filename}
                </Anchor>
                <Text size="xs" c="dimmed">
                  {formatBytes(a.size)} · <When iso={a.created_at} />
                </Text>
                <Group gap={4} mt={4}>
                  {showRecord && <RecordLink a={a} />}
                  {onAttach && (
                    <Button size="compact-xs" variant="light" leftSection={<IconPaperclip size={12} />} onClick={() => onAttach(a)}>
                      {attachLabel}
                    </Button>
                  )}
                  {onDetach && (
                    <Button size="compact-xs" variant="subtle" onClick={() => onDetach(a)}>
                      Detach
                    </Button>
                  )}
                  <Button size="compact-xs" variant="subtle" color="red" onClick={() => onDelete(a)}>
                    Delete
                  </Button>
                </Group>
              </div>
            </Group>
          </Card>
        ))}
      </SimpleGrid>
    );
  }
  return (
    <Table fz="sm" highlightOnHover>
      <Table.Tbody>
        {rows.map((a) => (
          <Table.Tr key={a.id} data-testid="attachment-row">
            <Table.Td w={56}>
              <Thumb a={a} url={api.attachmentUrl(co, a.id)} />
            </Table.Td>
            <Table.Td>
              <Anchor href={api.attachmentUrl(co, a.id)} target="_blank" rel="noreferrer" size="sm">
                {a.filename}
              </Anchor>
              <Text size="xs" c="dimmed">
                {a.mime} · {formatBytes(a.size)}
                {a.created_by ? ` · ${a.created_by}` : ''}
              </Text>
            </Table.Td>
            <Table.Td>
              <When iso={a.created_at} />
            </Table.Td>
            <Table.Td>{showRecord && <RecordLink a={a} />}</Table.Td>
            <Table.Td ta="right">
              <Group gap={4} justify="flex-end" wrap="nowrap">
                {onAttach && (
                  <Button size="compact-xs" variant="light" leftSection={<IconPaperclip size={12} />} onClick={() => onAttach(a)}>
                    {attachLabel}
                  </Button>
                )}
                {onDetach && (
                  <Button size="compact-xs" variant="subtle" onClick={() => onDetach(a)}>
                    Detach
                  </Button>
                )}
                <Tooltip label="Delete">
                  <ActionIcon variant="subtle" color="red" aria-label={`Delete ${a.filename}`} onClick={() => onDelete(a)}>
                    <IconTrash size={16} />
                  </ActionIcon>
                </Tooltip>
              </Group>
            </Table.Td>
          </Table.Tr>
        ))}
      </Table.Tbody>
    </Table>
  );
}

function Thumb({ a, url }: { a: AttachmentMeta; url: string }) {
  if (a.mime.startsWith('image/')) return <Image src={url} alt="" w={48} h={48} fit="cover" radius="sm" />;
  return (
    <Group w={48} h={48} justify="center" align="center" style={{ background: 'var(--mantine-color-default-hover)', borderRadius: 4 }}>
      <Text size="xs" c="dimmed">
        {a.mime === 'application/pdf' ? 'PDF' : (a.filename.split('.').pop() ?? 'file').toUpperCase().slice(0, 4)}
      </Text>
    </Group>
  );
}

type TargetKind = 'txn' | 'entity' | 'account' | 'item';

/**
 * Pick what an inbox file belongs to: a transaction (search by payee, memo or
 * number), or a name, account or item (QuickFill).
 */
function AttachToModal({ attachment, onClose, onPick, busy, error }: { attachment: AttachmentMeta | null; onClose(): void; onPick(to: AttachmentRecord): void; busy: boolean; error: unknown }) {
  const [text, setText] = useState('');
  const [kind, setKind] = useState<TargetKind>('txn');
  const [picked, setPicked] = useState<string | null>(null);
  const entities = useEntityIndex();
  const txns = useTxns({ text: text.trim() || null, descending: true, limit: 25 }, !!attachment && kind === 'txn');
  useEffect(() => {
    setText('');
    setPicked(null);
  }, [attachment, kind]);
  const Picker = kind === 'entity' ? NameQuickFill : kind === 'account' ? AccountQuickFill : ItemQuickFill;
  return (
    <Modal opened={!!attachment} onClose={onClose} title={`Attach "${attachment?.filename}" to…`} centered size="lg">
      <Stack gap="sm">
        <SegmentedControl
          size="xs"
          data={[
            { value: 'txn', label: 'Transaction' },
            { value: 'entity', label: 'Name' },
            { value: 'account', label: 'Account' },
            { value: 'item', label: 'Item' },
          ]}
          value={kind}
          onChange={(v) => setKind(v as TargetKind)}
        />
        {kind !== 'txn' && (
          <Group align="flex-end" gap="xs" wrap="nowrap">
            <Picker key={kind} label={kind === 'entity' ? 'Name' : kind === 'account' ? 'Account' : 'Item'} value={picked} onChange={setPicked} allowAddNew={false} showInactive autoFocus style={{ flex: 1 }} aria-label={`Attach to ${RECORD_NOUN[kind]}`} />
            <Button onClick={() => picked && onPick({ record_kind: kind, record_id: picked })} disabled={!picked} loading={busy}>
              Attach
            </Button>
          </Group>
        )}
        {kind === 'txn' && <TextInput placeholder="Payee, memo or number" value={text} onChange={(e) => setText(e.currentTarget.value)} data-autofocus aria-label="Find a transaction" />}
        {!!error && <Alert color="red">{describeError(error)}</Alert>}
        {kind === 'txn' && (
        <Table fz="sm" highlightOnHover className={classes.clickable}>
          <Table.Tbody>
            {(txns.data ?? []).map((t) => (
              <Table.Tr key={t.id} onClick={() => !busy && onPick({ record_kind: 'txn', record_id: t.id })} tabIndex={0} onKeyDown={(e) => e.key === 'Enter' && onPick({ record_kind: 'txn', record_id: t.id })} data-testid="attach-target">
                <Table.Td>{formatDateShort(t.date)}</Table.Td>
                <Table.Td>
                  {TXN_LABEL[t.txn_type]}
                  {t.number ? ` ${t.number}` : ''}
                </Table.Td>
                <Table.Td>{t.entity_id ? (entities.get(t.entity_id)?.full_name ?? '') : ''}</Table.Td>
                <Table.Td c="dimmed">{t.memo}</Table.Td>
                <Table.Td ta="right" className={classes.num}>
                  {formatMoney(Math.abs(t.amount))}
                </Table.Td>
              </Table.Tr>
            ))}
            {txns.data?.length === 0 && (
              <Table.Tr>
                <Table.Td colSpan={5}>
                  <Text c="dimmed" size="sm">
                    No transactions match.
                  </Text>
                </Table.Td>
              </Table.Tr>
            )}
          </Table.Tbody>
        </Table>
        )}
      </Stack>
    </Modal>
  );
}
