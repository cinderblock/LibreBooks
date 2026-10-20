// The transaction-form engine every form shares (Write Checks was the template;
// this is its state machine lifted out): the draft and its dirtiness, client
// validation then server errors mapped onto fields, Save & New / Save & Close,
// Delete and Void, optimistic concurrency (a stale edit_seq → the conflict
// dialog; a change pushed while editing → a banner, or a silent reload when
// nothing was typed), Previous/Next, the Journal, History and Memorize dialogs,
// and the QBD keys for all of it. A form supplies its pure model (`TxnFormSpec`)
// and renders its own fields.

import { Alert, Badge, Button, Group, Loader, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconPaperclip } from '@tabler/icons-react';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useApi, useCompanyId, useRecordChanges } from '~/api/context';
import { describeError, isApiError } from '~/api/errors';
import { useCompanyInfo, useDeleteTxn, useSaveTxn, useTxn, useTxns, useVoidTxn } from '~/api/hooks';
import { qk } from '~/api/keys';
import type { Txn } from '~/api/types';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { useEnterBehavior } from '~/lib/prefs';
import { useIsMobile } from '~/lib/useMobile';
import { takeTxn } from '~/windows/handoff';
import { paths } from '~/windows/paths';
import { useWindow, useWindows } from '~/windows/WindowsProvider';
import { ConflictModal, flushActiveField, FormActionBar, HistoryModal, PostingsModal, RemoteChangeBanner, ShortcutButton } from './common';
import { MemorizeModal } from './MemorizeModal';
import { hasErrors, noErrors, type FormErrors, type TxnFormSpec } from './model';

export interface TxnFormOptions<D> {
  spec: TxnFormSpec<D>;
  /** The saved transaction being edited, or null for a new one. */
  saved: Txn | null;
  /** A prefilled, unsaved transaction (from a memorized one or a bank feed line). */
  initial?: Txn | null;
  /** After Save & New: the form puts the caret where entry starts. */
  onSavedNew?(): void;
}

export interface TxnForm<D> {
  draft: D;
  saved: Txn | null;
  update(patch: Partial<D> | ((d: D) => D)): void;
  reset(to?: D): void;
  errors: FormErrors;
  setErrors(e: FormErrors): void;
  dirty: boolean;
  saving: boolean;
  save(mode: 'new' | 'close'): Promise<void>;
  remove(): Promise<void>;
  voidIt(): Promise<void>;
  /** Previous (-1) / Next (+1) transaction of this kind. */
  go(delta: 1 | -1): void;
  showJournal(): void;
  showHistory(): void;
  memorize(): void;
  mobile: boolean;
  enterBehavior: 'next-cell' | 'next-row' | 'save';
  /** "Someone else saved this" banner, to render at the top of the form. */
  banner: ReactNode;
  /** The form-wide error alert (null when none). */
  formErrors: ReactNode;
  /** The dialogs: render once, anywhere in the form. */
  chrome: ReactNode;
}

export function useTxnForm<D>({ spec, saved, initial = null, onSavedNew }: TxnFormOptions<D>): TxnForm<D> {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  const win = useWindow();
  const windows = useWindows();
  const mobile = useIsMobile();
  const [enterBehavior] = useEnterBehavior();
  const [draft, setDraft] = useState<D>(() => (saved ? spec.fromTxn(saved) : initial ? spec.fromTxn(initial) : spec.newDraft()));
  const [baseline, setBaseline] = useState(() => spec.fingerprint(draft));
  const [errors, setErrors] = useState<FormErrors>(noErrors());
  const [conflict, setConflict] = useState<Txn | null>(null);
  const [remoteChanged, setRemoteChanged] = useState(false);
  const [journalFor, setJournalFor] = useState<Txn | null>(null);
  const [historyFor, setHistoryFor] = useState<string | null>(null);
  const [memorizeFor, setMemorizeFor] = useState<Txn | null>(null);
  const latest = useRef(draft);
  latest.current = draft;
  const noun = typeof spec.noun === 'function' ? spec.noun(draft) : spec.noun;

  // A new form is dirty once anything is entered (a hand-off counts); a saved one once it differs.
  const dirty = saved ? spec.fingerprint(draft) !== baseline : !spec.isEmpty(draft);
  useEffect(() => win.setDirty(dirty), [dirty]); // eslint-disable-line react-hooks/exhaustive-deps

  const title = spec.title(draft, saved);
  useEffect(() => win.setTitle(title), [title]); // eslint-disable-line react-hooks/exhaustive-deps

  // Someone else saved this record: reload quietly with no edits here, else say so.
  useRecordChanges('txn', saved?.id, () => {
    if (!dirty) void qc.invalidateQueries({ queryKey: qk.txn(co, saved!.id) });
    else setRemoteChanged(true);
  });
  useEffect(() => {
    if (saved && !dirty && saved.edit_seq !== (spec.toTxn(draft).edit_seq ?? 0)) {
      const d = spec.fromTxn(saved);
      setDraft(d);
      setBaseline(spec.fingerprint(d));
    }
  }, [saved]); // eslint-disable-line react-hooks/exhaustive-deps

  const saveM = useSaveTxn();
  const del = useDeleteTxn();
  const voidM = useVoidTxn();

  const update = (patch: Partial<D> | ((d: D) => D)) => {
    setDraft((d) => (typeof patch === 'function' ? (patch as (d: D) => D)(d) : { ...d, ...patch }));
    if (hasErrors(errors)) setErrors(noErrors());
  };

  const reset = (to?: D) => {
    const next = to ?? (saved ? spec.fromTxn(saved) : spec.newDraft());
    setDraft(next);
    setBaseline(spec.fingerprint(next));
    setErrors(noErrors());
    setRemoteChanged(false);
  };

  const save = async (mode: 'new' | 'close', opts: { overwrite?: boolean } = {}) => {
    await flushActiveField();
    let d = latest.current;
    const v = spec.validate(d);
    if (hasErrors(v)) {
      setErrors(v);
      return;
    }
    let txn = spec.toTxn(d);
    if (opts.overwrite && saved) {
      // Take the server's current edit_seq and save ours over it.
      const current = await api.txn(co, txn.id);
      txn = { ...txn, edit_seq: current.txn.edit_seq };
    }
    try {
      const out = await saveM.mutateAsync(txn);
      notifications.show({ message: spec.savedMessage(out), color: 'teal' });
      if (mode === 'close') {
        win.close({ force: true });
      } else if (saved) {
        // Editing an existing record: Save & New opens a fresh form of this kind.
        win.close({ force: true });
        windows.open(spec.newPath);
      } else {
        d = latest.current;
        reset(spec.nextDraft(d));
        requestAnimationFrame(() => onSavedNew?.());
      }
    } catch (e) {
      if (isApiError(e) && e.isStaleConflict) setConflict(txn);
      else if (isApiError(e)) setErrors(spec.serverErrors(d, txn, e));
      else setErrors({ ...noErrors(), form: [describeError(e)] });
    }
  };

  const remove = async () => {
    if (!saved) return;
    if (!window.confirm(`Delete this ${noun}? This can't be undone.`)) return;
    try {
      await del.mutateAsync({ id: saved.id, editSeq: saved.edit_seq });
      notifications.show({ message: `${capitalize(noun)} deleted.`, color: 'gray' });
      win.close({ force: true });
    } catch (e) {
      setErrors({ ...noErrors(), form: [describeError(e)] });
    }
  };

  const voidIt = async () => {
    if (!saved) return;
    if (!window.confirm(`Void this ${noun}? Its amounts become zero; it stays in the books marked VOID.`)) return;
    try {
      await voidM.mutateAsync({ id: saved.id, editSeq: saved.edit_seq });
      notifications.show({ message: `${capitalize(noun)} voided.`, color: 'gray' });
    } catch (e) {
      setErrors({ ...noErrors(), form: [describeError(e)] });
    }
  };

  // Previous / Next (Page Up / Page Down), in date order within the kind.
  const siblingsQuery = spec.siblings?.(draft) ?? null;
  const siblings = useTxns(siblingsQuery ?? {}, !!siblingsQuery);
  const go = (delta: 1 | -1) => {
    const list = siblings.data ?? [];
    if (!list.length) return;
    const i = saved ? list.findIndex((s) => s.id === saved.id) : list.length;
    const target = list[i + delta];
    if (!target) {
      notifications.show({ message: delta < 0 ? `This is the first ${noun}.` : `This is the last ${noun}.`, color: 'gray' });
      return;
    }
    windows.open(`/txns/${encodeURIComponent(target.id)}`);
  };

  const enabled = win.isActiveNow;
  useShortcut('form.saveNew', () => void save('new'), { enabled });
  useShortcut('form.saveClose', () => void save('close'), { enabled });
  useShortcut('record.new', () => windows.open(spec.newPath), { enabled });
  useShortcut('txn.journal', () => setJournalFor(spec.toTxn(latest.current)), { enabled });
  useShortcut('record.history', () => (saved ? setHistoryFor(saved.id) : false), { enabled });
  useShortcut('record.print', () => window.print(), { enabled });
  useShortcut('record.delete', () => (saved ? void remove() : false), { enabled });
  useShortcut('txn.memorize', () => setMemorizeFor(spec.toTxn(latest.current)), { enabled });
  useShortcut('txn.prev', () => go(-1), { enabled });
  useShortcut('txn.next', () => go(1), { enabled });

  const banner = remoteChanged ? (
    <RemoteChangeBanner
      onReload={() => {
        setRemoteChanged(false);
        setBaseline(spec.fingerprint(draft)); // treat as clean so the reload effect applies
        void qc.invalidateQueries({ queryKey: qk.txn(co, saved!.id) });
      }}
    />
  ) : null;

  const formErrors =
    errors.form.length > 0 ? (
      <Alert color="red" role="alert">
        {errors.form.map((m) => (
          <div key={m}>{m}</div>
        ))}
      </Alert>
    ) : null;

  const chrome = (
    <>
      <ConflictModal
        opened={!!conflict}
        what={noun}
        onCancel={() => setConflict(null)}
        onReload={() => {
          setConflict(null);
          setBaseline(spec.fingerprint(draft));
          void qc.invalidateQueries({ queryKey: qk.txn(co, spec.toTxn(draft).id) });
        }}
        onOverwrite={() => {
          setConflict(null);
          void save('close', { overwrite: true });
        }}
      />
      <PostingsModal txn={journalFor} onClose={() => setJournalFor(null)} />
      <HistoryModal txnId={historyFor} onClose={() => setHistoryFor(null)} />
      <MemorizeModal txn={memorizeFor} onClose={() => setMemorizeFor(null)} />
    </>
  );

  return {
    draft,
    saved,
    update,
    reset,
    errors,
    setErrors,
    dirty,
    saving: saveM.isPending,
    save: (mode) => save(mode),
    remove,
    voidIt,
    go,
    showJournal: () => setJournalFor(spec.toTxn(latest.current)),
    showHistory: () => saved && setHistoryFor(saved.id),
    memorize: () => setMemorizeFor(spec.toTxn(latest.current)),
    mobile,
    enterBehavior,
    banner,
    formErrors,
    chrome,
  };
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/**
 * The standard action bar: Delete, Void, Journal, Revert/Clear (desktop), then
 * Save & New and Save & Close, each with its key. Forms add buttons through
 * `extra` (a Print, a "Payments to deposit"…).
 */
export function TxnFormActions<D>({ form, extra, hideVoid }: { form: TxnForm<D>; extra?: ReactNode; hideVoid?: boolean }) {
  const { mobile, saved, dirty, saving } = form;
  const { open } = useWindows();
  return (
    <FormActionBar mobile={mobile}>
      {!mobile && saved && (
        <ShortcutButton variant="subtle" color="red" command="record.delete" onClick={() => void form.remove()}>
          Delete
        </ShortcutButton>
      )}
      {saved && (
        // Receipts go on the saved transaction: the Attachments window scoped to it.
        <Tooltip label="Attachments (receipts, documents)" openDelay={600}>
          <Button variant="default" aria-label="Attachments" px="xs" onClick={() => open(paths.attachments({ kind: 'txn', id: saved.id }))}>
            <IconPaperclip size={16} />
          </Button>
        </Tooltip>
      )}
      {!mobile && saved && !hideVoid && !saved.is_void && (
        <Button variant="subtle" color="orange" onClick={() => void form.voidIt()}>
          Void
        </Button>
      )}
      {!mobile && (
        <ShortcutButton variant="default" command="txn.journal" onClick={form.showJournal}>
          Journal
        </ShortcutButton>
      )}
      {!mobile && (
        <ShortcutButton variant="default" command="txn.memorize" onClick={form.memorize}>
          Memorize
        </ShortcutButton>
      )}
      {extra}
      {!mobile && (
        <Button variant="default" onClick={() => form.reset()} disabled={!dirty}>
          {saved ? 'Revert' : 'Clear'}
        </Button>
      )}
      <ShortcutButton variant="default" command="form.saveNew" loading={saving} onClick={() => void form.save('new')}>
        Save &amp; New
      </ShortcutButton>
      <ShortcutButton command="form.saveClose" loading={saving} onClick={() => void form.save('close')}>
        Save &amp; Close
      </ShortcutButton>
    </FormActionBar>
  );
}

/** The VOID badge and the non-posting note a form shows beside its title. */
export function FormBadges({ saved, nonPosting }: { saved: Txn | null; nonPosting?: boolean }) {
  return (
    <Group gap="xs">
      {saved?.is_void && <Badge color="red">VOID</Badge>}
      {nonPosting && (
        <Badge variant="light" color="gray" title="Non-posting: this form moves no money until it becomes a bill, check or invoice.">
          Non-posting
        </Badge>
      )}
    </Group>
  );
}

/**
 * Load the transaction a form window edits (`txnId`) or the hand-off it starts
 * from (`?draft=`), then render the form. One place, so every form opens the
 * same way.
 */
export function FormLoader({ txnId, draftToken, render }: { txnId?: string | null; draftToken?: string | null; render(saved: Txn | null, initial: Txn | null): ReactNode }) {
  const loaded = useTxn(txnId);
  // Taken once; React StrictMode runs initializers twice, so keep what we took.
  const [initial] = useState(() => takeTxn(draftToken));
  if (txnId && loaded.isLoading) return <Loader m="xl" />;
  if (txnId && loaded.error) return <Alert color="red" m="md">{describeError(loaded.error)}</Alert>;
  return <>{render(loaded.data?.txn ?? null, initial)}</>;
}

/** The company's preferences, for forms that default from them. */
export function useFormPrefs() {
  const { data: info } = useCompanyInfo();
  return info?.preferences ?? null;
}
