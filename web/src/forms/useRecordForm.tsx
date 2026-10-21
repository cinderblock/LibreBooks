// The list-record editors' engine: what `useTxnForm` is to a transaction form,
// for an account, a name, an item, a class, terms, a payment method or a sales
// tax code. The same shape on purpose: a pure spec (draft ↔ record, validate,
// fingerprint, server-error mapping, the next draft after Save & New), the
// engine does the rest: dirtiness, Save & New / Save & Close (Alt+S / Alt+A),
// Revert/Clear, Delete (Ctrl+D) with the server's "in use" refusal shown on
// the form, a stale `edit_seq` → the conflict dialog (Reload theirs / Save
// mine anyway / Keep editing), a change pushed while editing → the banner (or a
// silent reload with nothing typed), and the paperclip for records the
// Attachments window can target.

import { Alert, Button, Loader, Tooltip } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconPaperclip } from '@tabler/icons-react';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import type { ApiClient } from '~/api/client';
import { useApi, useCompanyId, useRecordChanges } from '~/api/context';
import { describeError, isApiError, type ApiError } from '~/api/errors';
import { useDeleteRecord, useRecord, useSaveRecord } from '~/api/hooks';
import { eventKindOf, qk } from '~/api/keys';
import type { CompanyId, ListName, ListRecords } from '~/api/types';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { useIsMobile } from '~/lib/useMobile';
import { paths } from '~/windows/paths';
import { useWindow, useWindows } from '~/windows/WindowsProvider';
import { ConflictModal, flushActiveField, FormActionBar, RemoteChangeBanner, ShortcutButton } from './common';
import { hasErrors, noErrors, type FormErrors } from './model';

/** What `useRecordForm` needs from an editor's pure model. */
export interface RecordFormSpec<L extends ListName, D> {
  list: L;
  /** "account", "customer", "item": toasts and confirmations (a function when the draft decides). */
  noun: string | ((d: D) => string);
  /** A fresh draft; `params` are the window's (`kind`, `type`, `parent` presets). */
  newDraft(params: Record<string, string>): D;
  fromRecord(r: ListRecords[L]): D;
  toRecord(d: D): ListRecords[L];
  validate(d: D): FormErrors;
  /** Nothing entered yet (closing a new form needn't ask). */
  isEmpty(d: D): boolean;
  /** A stable fingerprint of what the user can change, for dirty tracking of saved records. */
  fingerprint(d: D): string;
  /** Put a server error next to the field it's about. */
  serverErrors(d: D, err: ApiError): FormErrors;
  /** The draft after Save & New (keeps the kind or type, as QBD does). */
  nextDraft(d: D): D;
  /** Company-relative path of a fresh editor of this kind (Save & New after editing). */
  newPath(d: D): string;
  title(d: D, saved: ListRecords[L] | null): string;
  savedMessage(r: ListRecords[L]): string;
  /** `entity`, `account` or `item`: the Attachments window can hold this record's files. */
  attachmentKind?: 'entity' | 'account' | 'item';
}

export interface RecordFormOptions<L extends ListName, D> {
  spec: RecordFormSpec<L, D>;
  /** The saved record being edited, or null for a new one. */
  saved: ListRecords[L] | null;
  /** The window's params (`kind`, `type`, `parent`). */
  params: Record<string, string>;
  /** After Save & New: put the caret where entry starts. */
  onSavedNew?(): void;
  /**
   * Runs once the record is saved (a new account's opening-balance entry). The
   * record stays saved if this fails; the error shows on the form.
   */
  afterSave?(d: D, saved: ListRecords[L], api: ApiClient, co: CompanyId): Promise<void>;
}

export interface RecordForm<L extends ListName, D> {
  draft: D;
  saved: ListRecords[L] | null;
  update(patch: Partial<D> | ((d: D) => D)): void;
  reset(to?: D): void;
  errors: FormErrors;
  setErrors(e: FormErrors): void;
  dirty: boolean;
  saving: boolean;
  save(mode: 'new' | 'close'): Promise<void>;
  remove(): Promise<void>;
  mobile: boolean;
  /** Where the paperclip goes, or null (a new record, or a kind without attachments). */
  attachmentsPath: string | null;
  /** The "someone else saved this" banner, to render at the top of the form. */
  banner: ReactNode;
  /** The form-wide error alert (null when none). */
  formErrors: ReactNode;
  /** The dialogs: render once, anywhere in the form. */
  chrome: ReactNode;
  /**
   * QBD's Enter: spread onto the form's root so Enter in a field moves to the
   * next one (Tab's twin), leaving fields that use Enter themselves alone.
   */
  enterProps: { onKeyDown(e: KeyboardEvent<HTMLElement>): void };
}

export function useRecordForm<L extends ListName, D>({ spec, saved, params, onSavedNew, afterSave }: RecordFormOptions<L, D>): RecordForm<L, D> {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  const win = useWindow();
  const windows = useWindows();
  const mobile = useIsMobile();
  const [draft, setDraft] = useState<D>(() => (saved ? spec.fromRecord(saved) : spec.newDraft(params)));
  const [baseline, setBaseline] = useState(() => spec.fingerprint(draft));
  const [errors, setErrors] = useState<FormErrors>(noErrors());
  const [conflict, setConflict] = useState(false);
  const [remoteChanged, setRemoteChanged] = useState(false);
  const latest = useRef(draft);
  latest.current = draft;
  const noun = typeof spec.noun === 'function' ? spec.noun(draft) : spec.noun;

  const dirty = saved ? spec.fingerprint(draft) !== baseline : !spec.isEmpty(draft);
  useEffect(() => win.setDirty(dirty), [dirty]); // eslint-disable-line react-hooks/exhaustive-deps

  const title = spec.title(draft, saved);
  useEffect(() => win.setTitle(title), [title]); // eslint-disable-line react-hooks/exhaustive-deps

  // Someone else saved this record: reload quietly with no edits here, else say so.
  const recordKey = saved ? qk.record(co, spec.list, saved.id) : null;
  useRecordChanges(eventKindOf(spec.list), saved?.id, () => {
    if (!dirty && recordKey) void qc.invalidateQueries({ queryKey: recordKey });
    else setRemoteChanged(true);
  });
  useEffect(() => {
    if (saved && !dirty && saved.edit_seq !== spec.toRecord(draft).edit_seq) {
      const d = spec.fromRecord(saved);
      setDraft(d);
      setBaseline(spec.fingerprint(d));
    }
  }, [saved]); // eslint-disable-line react-hooks/exhaustive-deps

  const saveM = useSaveRecord(spec.list);
  const del = useDeleteRecord(spec.list);

  const update = (patch: Partial<D> | ((d: D) => D)) => {
    setDraft((d) => (typeof patch === 'function' ? (patch as (d: D) => D)(d) : { ...d, ...patch }));
    if (hasErrors(errors)) setErrors(noErrors());
  };

  const reset = (to?: D) => {
    const next = to ?? (saved ? spec.fromRecord(saved) : spec.newDraft(params));
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
    let rec = spec.toRecord(d);
    if (opts.overwrite && saved) {
      // Take the server's current edit_seq and save ours over it.
      const current = await api.getRecord(co, spec.list, rec.id);
      rec = { ...rec, edit_seq: current.edit_seq };
    }
    let out: ListRecords[L];
    try {
      out = await saveM.mutateAsync(rec);
    } catch (e) {
      if (isApiError(e) && e.isStaleConflict) setConflict(true);
      else if (isApiError(e)) setErrors(spec.serverErrors(d, e));
      else setErrors({ ...noErrors(), form: [describeError(e)] });
      return;
    }
    notifications.show({ message: spec.savedMessage(out), color: 'teal' });
    if (afterSave) {
      try {
        await afterSave(d, out, api, co);
      } catch (e) {
        // The record is saved; what follows it isn't. Show the editor for the saved record with the problem on it.
        setErrors({ ...noErrors(), form: [describeError(e)] });
        if (!saved) {
          win.close({ force: true });
          windows.open(paths.record(spec.list, out.id));
        }
        return;
      }
    }
    if (mode === 'close') {
      win.close({ force: true });
    } else if (saved) {
      // Editing an existing record: Save & New opens a fresh editor of this kind.
      win.close({ force: true });
      windows.open(spec.newPath(d));
    } else {
      d = latest.current;
      reset(spec.nextDraft(d));
      requestAnimationFrame(() => onSavedNew?.());
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
      // 409 `in_use`: the server says what still refers to it; QBD's answer is "make it inactive".
      setErrors({ ...noErrors(), form: [describeError(e)] });
    }
  };

  const enabled = win.isActiveNow;
  useShortcut('form.saveNew', () => void save('new'), { enabled });
  useShortcut('form.saveClose', () => void save('close'), { enabled });
  useShortcut('record.new', () => windows.open(spec.newPath(latest.current)), { enabled });
  useShortcut('record.delete', () => (saved ? void remove() : false), { enabled });

  const banner = remoteChanged ? (
    <RemoteChangeBanner
      what={noun}
      onReload={() => {
        setRemoteChanged(false);
        setBaseline(spec.fingerprint(draft)); // treat as clean so the reload effect applies
        if (recordKey) void qc.invalidateQueries({ queryKey: recordKey });
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
    <ConflictModal
      opened={conflict}
      what={noun}
      onCancel={() => setConflict(false)}
      onReload={() => {
        setConflict(false);
        setBaseline(spec.fingerprint(draft));
        if (recordKey) void qc.invalidateQueries({ queryKey: recordKey });
      }}
      onOverwrite={() => {
        setConflict(false);
        void save('close', { overwrite: true });
      }}
    />
  );

  const attachmentsPath = saved && spec.attachmentKind ? paths.attachments({ kind: spec.attachmentKind, id: saved.id }) : null;

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
    mobile,
    attachmentsPath,
    banner,
    formErrors,
    chrome,
    enterProps: { onKeyDown: enterMovesOn },
  };
}

/**
 * Enter in a plain field moves to the next field, as QBD's "Enter moves between
 * fields" does on its dialogs. Fields that use Enter themselves (an open
 * QuickFill list, the amount calculator, a date field, a select's dropdown)
 * have already handled it (`defaultPrevented`) or are left alone (textareas,
 * buttons, checkboxes, open comboboxes).
 */
function enterMovesOn(e: KeyboardEvent<HTMLElement>) {
  if (e.key !== 'Enter' || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
  const el = e.target as HTMLElement;
  if (!(el instanceof HTMLInputElement)) return;
  if (el.type === 'checkbox' || el.type === 'radio' || el.type === 'button' || el.type === 'submit') return;
  if (el.getAttribute('aria-expanded') === 'true') return;
  const root = e.currentTarget;
  const fields = [...root.querySelectorAll<HTMLElement>('input, textarea, select, button, [tabindex]')].filter((f) => f.tabIndex >= 0 && !f.hasAttribute('disabled') && !(f instanceof HTMLInputElement && f.type === 'hidden') && f.offsetParent !== null);
  const i = fields.indexOf(el);
  if (i < 0) return;
  e.preventDefault();
  el.blur();
  const next = fields[i + 1];
  if (next) {
    next.focus();
    if (next instanceof HTMLInputElement && next.type === 'text') next.select();
  }
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/**
 * The editors' action bar: Delete, the paperclip (saved records the
 * Attachments window can hold), Revert/Clear, Save & New and Save & Close,
 * each with its key.
 */
export function RecordFormActions<L extends ListName, D>({ form, extra }: { form: RecordForm<L, D>; extra?: ReactNode }) {
  const { mobile, saved, dirty, saving } = form;
  const { open } = useWindows();
  return (
    <FormActionBar mobile={mobile}>
      {!mobile && saved && (
        <ShortcutButton variant="subtle" color="red" command="record.delete" onClick={() => void form.remove()}>
          Delete
        </ShortcutButton>
      )}
      {form.attachmentsPath && (
        <Tooltip label="Attachments (documents, photos)" openDelay={600}>
          <Button variant="default" aria-label="Attachments" px="xs" onClick={() => open(form.attachmentsPath!)}>
            <IconPaperclip size={16} />
          </Button>
        </Tooltip>
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

/**
 * Load the record an editor window edits (`recordId`), then render the editor.
 * One place, so every editor opens the same way (and a missing record says so).
 */
export function RecordLoader<L extends ListName>({ list, recordId, render }: { list: L; recordId?: string | null; render(saved: ListRecords[L] | null): ReactNode }) {
  const loaded = useRecord(list, recordId);
  if (recordId && loaded.isLoading) return <Loader m="xl" />;
  if (recordId && loaded.error) return <Alert color="red" m="md">{describeError(loaded.error)}</Alert>;
  if (recordId && !loaded.data) return null;
  return <>{render(loaded.data ?? null)}</>;
}
