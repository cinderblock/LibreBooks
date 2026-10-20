// Attachments (Company → Attachments, and the paperclip on a form): receipts
// and documents. Upload a file or take a photo (`<input capture>` opens the
// camera on a phone); what isn't attached to anything yet waits in the inbox
// until it's attached to a transaction. Opened from a form, the window shows
// that record's files first and offers to attach inbox items to it.

import { ActionIcon, Alert, Anchor, Badge, Button, Card, FileButton, Group, Image, Modal, SimpleGrid, Stack, Table, Text, TextInput, Title, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconCamera, IconExternalLink, IconPaperclip, IconTrash, IconUpload } from '@tabler/icons-react';
import { useMutation } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { describeError } from '~/api/errors';
import { useAttachments, useEntityIndex, useInvalidate, useTxns } from '~/api/hooks';
import { qk } from '~/api/keys';
import { asNumber, type AttachmentMeta, type AttachmentRecord } from '~/api/types';
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
  const inbox = useAttachments({});
  const mine = useAttachments(record ? { record_kind: record.record_kind, record_id: record.record_id } : {}, !!record);
  const [attaching, setAttaching] = useState<AttachmentMeta | null>(null);
  const [deleting, setDeleting] = useState<AttachmentMeta | null>(null);
  const camera = useRef<HTMLInputElement>(null);
  useEffect(() => win.setTitle(record ? 'Attachments (this transaction)' : 'Attachments'), [record?.record_id]); // eslint-disable-line react-hooks/exhaustive-deps

  const refresh = () => invalidate(qk.attachmentsAll(co));
  const upload = useMutation({
    mutationFn: async (files: File[]) => {
      const out: AttachmentMeta[] = [];
      for (const f of files) out.push(await api.uploadAttachment(co, f, record));
      return out;
    },
    onSuccess: (metas) => {
      refresh();
      notifications.show({ message: metas.length === 1 ? `Added ${metas[0]!.filename}${record ? ' to this transaction' : ' to the inbox'}.` : `Added ${metas.length} files.`, color: 'teal' });
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
            {record ? 'Files on this transaction, and the inbox to attach from.' : 'Receipts and documents. Files land in the inbox until they are attached to a transaction.'}
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
            On this transaction
          </Text>
          <AttachmentTable rows={mine.data ?? []} loading={mine.isLoading} empty="Nothing attached yet. Add a file above, or attach one from the inbox below." onDetach={(a) => attach.mutate({ a, to: null })} onDelete={setDeleting} />
        </Card>
      )}

      <Card withBorder>
        <Group gap="xs" mb="xs">
          <Text fw={600}>Inbox</Text>
          {(inbox.data?.length ?? 0) > 0 && <Badge variant="light">{inbox.data!.length}</Badge>}
        </Group>
        <AttachmentTable
          rows={inbox.data ?? []}
          loading={inbox.isLoading}
          empty="The inbox is empty."
          onAttach={record ? (a) => attach.mutate({ a, to: record }) : setAttaching}
          attachLabel={record ? 'Attach here' : 'Attach to…'}
          onDelete={setDeleting}
        />
      </Card>

      <AttachToModal attachment={attaching} onClose={() => setAttaching(null)} onPick={(to) => attaching && attach.mutate({ a: attaching, to })} busy={attach.isPending} error={attach.error} />
      <Confirm opened={!!deleting} title={`Delete "${deleting?.filename}"?`} confirmLabel="Delete" loading={remove.isPending} onConfirm={() => deleting && remove.mutate(deleting)} onClose={() => setDeleting(null)}>
        <Text size="sm">The file is removed from the books for good.</Text>
        {remove.error && <Alert color="red" mt="sm">{describeError(remove.error)}</Alert>}
      </Confirm>
    </Stack>
  );
}

function AttachmentTable({ rows, loading, empty, onAttach, attachLabel, onDetach, onDelete }: { rows: AttachmentMeta[]; loading: boolean; empty: string; onAttach?(a: AttachmentMeta): void; attachLabel?: string; onDetach?(a: AttachmentMeta): void; onDelete(a: AttachmentMeta): void }) {
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const open = useOpenWindow();
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
                  {formatBytes(asNumber(a.size))} · <When iso={a.created_at} />
                </Text>
                <Group gap={4} mt={4}>
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
                {a.mime} · {formatBytes(asNumber(a.size))}
                {a.created_by ? ` · ${a.created_by}` : ''}
              </Text>
            </Table.Td>
            <Table.Td>
              <When iso={a.created_at} />
            </Table.Td>
            <Table.Td>
              {a.record_kind === 'txn' && a.record_id && (
                <Button size="compact-xs" variant="subtle" rightSection={<IconExternalLink size={12} />} onClick={() => open(paths.txn(a.record_id!))}>
                  Open transaction
                </Button>
              )}
            </Table.Td>
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

/** Pick the transaction an inbox file belongs to: search by payee, memo or number. */
function AttachToModal({ attachment, onClose, onPick, busy, error }: { attachment: AttachmentMeta | null; onClose(): void; onPick(to: AttachmentRecord): void; busy: boolean; error: unknown }) {
  const [text, setText] = useState('');
  const entities = useEntityIndex();
  const txns = useTxns({ text: text.trim() || null, descending: true, limit: 25 }, !!attachment);
  useEffect(() => setText(''), [attachment]);
  return (
    <Modal opened={!!attachment} onClose={onClose} title={`Attach "${attachment?.filename}" to…`} centered size="lg">
      <Stack gap="sm">
        <TextInput placeholder="Payee, memo or number" value={text} onChange={(e) => setText(e.currentTarget.value)} data-autofocus aria-label="Find a transaction" />
        {!!error && <Alert color="red">{describeError(error)}</Alert>}
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
      </Stack>
    </Modal>
  );
}
