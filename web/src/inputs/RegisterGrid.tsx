// RegisterGrid: an account register as QBD draws it. Two lines per transaction
// (Date · Number · Payee · Payment · ✓ · Deposit · Balance, then Type · Account
// · Memo) or one line in "1-Line" mode, newest at the bottom with a blank line
// to type a new one, and virtualized so 100,000 lines scroll as smoothly as 100.
//
// Keys (the grid has focus): ↑/↓ move, Page Up/Down page, Ctrl+Home/End jump,
// Enter opens the transaction, Space toggles cleared. In an entry line (the
// bottom one, or a saved line being edited in place), Tab moves between fields,
// Enter records and Esc cancels an edit.
//
// Marks: ✓ cleared (reconciled), * pending (being reconciled), ⚡ downloaded
// from the bank (matched or added by a bank feed).
//
// On phones: a list of tappable cards (newest first), with a cleared toggle.

import { ActionIcon, Button, Group, Text, TextInput, UnstyledButton } from '@mantine/core';
import { IconBolt, IconCircleCheck, IconCircleDashed } from '@tabler/icons-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { Account, RegisterLine } from '~/api/types';
import { naturalBalance, registerColumns, registerSplit } from '~/lib/accountTypes';
import { formatDateShort, todayIso } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { TXN_REGISTER_CODE } from '~/lib/txnTypes';
import { blankEntry, isBlankEntry, type RegisterEntry } from '~/forms/registerEntry';
import { DateField } from './DateField';
import { MoneyField } from './MoneyField';
import { AccountQuickFill, NameQuickFill } from './pickers';
import classes from './RegisterGrid.module.css';

/** `RegisterLine` plus the bank-feed mark the server added (QBD's lightning bolt). */
type Line = RegisterLine & { downloaded?: boolean };

export interface EditState {
  txnId: string;
  entry: RegisterEntry;
}

export interface RegisterGridProps {
  account: Account;
  lines: RegisterLine[];
  onOpen(line: RegisterLine): void;
  onToggleCleared(line: RegisterLine): void;
  /** Record a typed entry; resolve with an error message, or null on success. */
  onRecord?(entry: RegisterEntry): Promise<{ field: string; message: string } | null>;
  /** A saved line being edited in place, and how to finish. */
  editing?: EditState | null;
  onSaveEdit?(entry: RegisterEntry): Promise<{ field: string; message: string } | null>;
  onCancelEdit?(): void;
  /** One row per transaction instead of QBD's two. */
  oneLine?: boolean;
  mobile?: boolean;
  /** Select this transaction once lines load (returning from a form). */
  initialTxnId?: string | null;
  onSelectionChange?(line: RegisterLine | null): void;
}

const ROW = 26; // one of the two lines, px

function Marks({ line }: { line: Line }) {
  return (
    <>
      {line.downloaded && (
        <span aria-label="Downloaded from the bank" title="Downloaded from the bank" className={classes.bolt}>
          <IconBolt size={12} />
        </span>
      )}
      {line.cleared === 'Cleared' ? <span aria-label="Cleared">✓</span> : line.cleared === 'Pending' ? <span aria-label="Pending">*</span> : null}
    </>
  );
}

export function RegisterGrid(props: RegisterGridProps) {
  return props.mobile ? <MobileRegister {...props} /> : <DesktopRegister {...props} />;
}

function DesktopRegister({ account, lines, onOpen, onToggleCleared, onRecord, editing, onSaveEdit, onCancelEdit, oneLine = false, initialTxnId, onSelectionChange }: RegisterGridProps) {
  const cols = registerColumns(account.account_type);
  const scrollEl = useRef<HTMLDivElement>(null);
  const hasEntry = !!onRecord;
  const count = lines.length + (hasEntry ? 1 : 0);
  const [selected, setSelected] = useState(() => Math.max(0, count - 1));
  const didInitialScroll = useRef(false);
  const rowH = oneLine ? ROW : ROW * 2;
  const editingIndex = editing ? lines.findIndex((l) => l.txn_id === editing.txnId) : -1;

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollEl.current,
    estimateSize: (i) => ((hasEntry && i === lines.length) || i === editingIndex ? ROW * 2 + 8 : rowH),
    overscan: 10,
  });
  useEffect(() => virtualizer.measure(), [oneLine, editingIndex]); // eslint-disable-line react-hooks/exhaustive-deps

  // QBD opens a register scrolled to the bottom (today's end), or on the
  // transaction you came from.
  // A window opened in the background (or just shown) has no size yet, and a
  // scroll then is lost; so wait until the list has a height.
  useLayoutEffect(() => {
    const el = scrollEl.current;
    if (didInitialScroll.current || count === 0 || !el) return;
    const i = initialTxnId ? lines.findIndex((l) => l.txn_id === initialTxnId) : -1;
    const target = i >= 0 ? i : count - 1;
    setSelected(target);
    const scroll = () => {
      if (didInitialScroll.current || el.clientHeight === 0) return;
      didInitialScroll.current = true;
      // The spacer already has the full height, so the DOM can scroll now even
      // though the virtualizer hasn't measured its viewport; refine next frame.
      if (i < 0) el.scrollTop = el.scrollHeight;
      requestAnimationFrame(() => virtualizer.scrollToIndex(target, { align: i >= 0 ? 'center' : 'end' }));
    };
    scroll();
    if (didInitialScroll.current) return;
    const ro = new ResizeObserver(scroll);
    ro.observe(el);
    return () => ro.disconnect();
  }, [count]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    onSelectionChange?.(lines[selected] ?? null);
  }, [selected, lines]); // eslint-disable-line react-hooks/exhaustive-deps

  const select = (i: number) => {
    const clamped = Math.max(0, Math.min(count - 1, i));
    setSelected(clamped);
    virtualizer.scrollToIndex(clamped, { align: 'auto' });
  };

  const page = Math.max(1, Math.floor((scrollEl.current?.clientHeight ?? 400) / rowH) - 1);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented) return;
    // Keys typed inside an entry line's fields belong to those fields.
    if ((e.target as HTMLElement).closest('[data-entry]')) return;
    const line = lines[selected];
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        select(selected + 1);
        break;
      case 'ArrowUp':
        e.preventDefault();
        select(selected - 1);
        break;
      case 'PageDown':
        e.preventDefault();
        select(selected + page);
        break;
      case 'PageUp':
        e.preventDefault();
        select(selected - page);
        break;
      case 'Home':
      case 'End':
        if (e.ctrlKey || e.metaKey) {
          e.preventDefault();
          select(e.key === 'Home' ? 0 : count - 1);
        }
        break;
      case 'Enter':
        e.preventDefault();
        if (line) onOpen(line);
        else if (hasEntry) scrollEl.current?.querySelector<HTMLElement>('[data-entry] input')?.focus();
        break;
      case ' ':
        if (line) {
          e.preventDefault();
          onToggleCleared(line);
        }
        break;
    }
  };

  const activeId = `reg-row-${selected}`;
  const r1 = oneLine ? classes.one : classes.r1;
  return (
    <div className={classes.wrap} data-one-line={oneLine || undefined}>
      <div className={classes.head} role="row">
        <div className={r1}>
          <span>Date</span>
          <span>Number</span>
          <span>Payee</span>
          {oneLine && <span>Account</span>}
          {oneLine && <span>Memo</span>}
          <span className={classes.num}>{cols.left}</span>
          <span className={classes.chk}>✓</span>
          <span className={classes.num}>{cols.right}</span>
          <span className={classes.num}>Balance</span>
        </div>
        {!oneLine && (
          <div className={classes.r2}>
            <span />
            <span>Type</span>
            <span>Account</span>
            <span className={classes.memo}>Memo</span>
          </div>
        )}
      </div>
      <div
        ref={scrollEl}
        className={classes.scroll}
        tabIndex={0}
        role="grid"
        aria-label={`${account.full_name} register`}
        aria-rowcount={count}
        aria-activedescendant={activeId}
        onKeyDown={onKeyDown}
        data-testid="register-grid"
      >
        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {virtualizer.getVirtualItems().map((vi) => {
            const style = { position: 'absolute' as const, top: 0, left: 0, right: 0, transform: `translateY(${vi.start}px)`, height: vi.size };
            if (hasEntry && vi.index === lines.length) {
              return (
                <div key="entry" style={style} className={classes.entry} data-entry id={`reg-row-${vi.index}`}>
                  <EntryRow account={account} onRecord={onRecord!} onFocus={() => setSelected(vi.index)} />
                </div>
              );
            }
            const l = lines[vi.index]! as Line;
            if (editing && vi.index === editingIndex && onSaveEdit && onCancelEdit) {
              return (
                <div key={`edit-${l.txn_id}`} style={style} className={classes.entry} data-entry data-editing id={`reg-row-${vi.index}`}>
                  <EntryRow account={account} initial={editing.entry} onRecord={onSaveEdit} onCancel={onCancelEdit} onFocus={() => setSelected(vi.index)} />
                </div>
              );
            }
            const { left, right } = registerSplit(account.account_type, l.amount);
            return (
              <div
                key={l.txn_id}
                id={`reg-row-${vi.index}`}
                role="row"
                aria-selected={vi.index === selected}
                className={classes.row}
                data-selected={vi.index === selected || undefined}
                data-void={l.is_void || undefined}
                data-odd={vi.index % 2 === 1 || undefined}
                style={style}
                onClick={() => {
                  setSelected(vi.index);
                  scrollEl.current?.focus({ preventScroll: true });
                }}
                onDoubleClick={() => onOpen(l)}
              >
                <div className={r1}>
                  <span>{formatDateShort(l.date)}</span>
                  <span>
                    {l.number}
                    {oneLine && <span className={classes.type}> {TXN_REGISTER_CODE[l.txn_type]}</span>}
                  </span>
                  <span className={classes.ellipsis}>{l.payee}</span>
                  {oneLine && <span className={classes.ellipsis}>{l.is_split ? '-split-' : l.other_account}</span>}
                  {oneLine && <span className={classes.ellipsis}>{l.is_void ? `VOID: ${l.memo ?? ''}` : l.memo}</span>}
                  <span className={classes.num}>{left ? formatMoney(left) : ''}</span>
                  <UnstyledButton
                    className={classes.chk}
                    tabIndex={-1}
                    aria-label={`Cleared: ${l.cleared}. Toggle.`}
                    onClick={(e) => {
                      e.stopPropagation();
                      onToggleCleared(l);
                    }}
                  >
                    <Marks line={l} />
                  </UnstyledButton>
                  <span className={classes.num}>{right ? formatMoney(right) : ''}</span>
                  <span className={classes.num} data-negative={naturalBalance(account.account_type, l.balance) < 0 || undefined}>
                    {formatMoney(naturalBalance(account.account_type, l.balance))}
                  </span>
                </div>
                {!oneLine && (
                  <div className={classes.r2}>
                    <span />
                    <span className={classes.type}>{TXN_REGISTER_CODE[l.txn_type]}</span>
                    <span className={classes.ellipsis}>{l.is_split ? '-split-' : l.other_account}</span>
                    <span className={`${classes.memo} ${classes.ellipsis}`}>{l.is_void ? `VOID: ${l.memo ?? ''}` : l.memo}</span>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function EntryRow({ account, onRecord, onCancel, initial, onFocus }: { account: Account; onRecord: NonNullable<RegisterGridProps['onRecord']>; onCancel?(): void; initial?: RegisterEntry; onFocus(): void }) {
  const cols = registerColumns(account.account_type);
  const [entry, setEntry] = useState<RegisterEntry>(() => initial ?? blankEntry(todayIso()));
  const [error, setError] = useState<{ field: string; message: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const dateRef = useRef<HTMLInputElement>(null);
  const latest = useRef(entry);
  latest.current = entry;
  const editing = !!initial;
  useEffect(() => {
    if (editing) dateRef.current?.focus();
  }, [editing]);
  const set = (patch: Partial<RegisterEntry>) => {
    setEntry((e) => ({ ...e, ...patch }));
    setError(null);
  };
  const record = async () => {
    // Let the focused field commit (amount arithmetic, typed date) first.
    (document.activeElement as HTMLElement | null)?.blur();
    await new Promise((r) => setTimeout(r, 0));
    setBusy(true);
    const err = await onRecord(latest.current);
    setBusy(false);
    if (err) setError(err);
    else if (!editing) {
      setEntry((e) => ({ ...blankEntry(e.date) }));
      dateRef.current?.focus();
    }
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      void record();
    } else if (e.key === 'Escape' && onCancel) {
      // Ours, not the window's close.
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    }
  };
  const errFor = (f: string) => (error?.field === f ? error.message : undefined);
  return (
    <div className={classes.entryInner} onKeyDown={onKeyDown} onFocus={onFocus}>
      <div className={classes.r1}>
        <DateField variant="cell" size="xs" value={entry.date} onChange={(d) => d && set({ date: d })} inputRef={dateRef} aria-label={editing ? 'Date' : 'New transaction date'} />
        <TextInput variant="unstyled" size="xs" classNames={{ input: 'lb-cell-text' }} value={entry.number} onChange={(e) => set({ number: e.currentTarget.value })} aria-label="Number" placeholder="Number" />
        <NameQuickFill variant="cell" size="xs" value={entry.payee_id} onChange={(id) => set({ payee_id: id })} aria-label="Payee" placeholder="Payee" error={errFor('payee')} />
        <MoneyField variant="cell" size="xs" blankZero value={entry.left} onChange={(v) => set({ left: v })} aria-label={cols.left} placeholder={cols.left} error={errFor('amount')} />
        <span className={classes.chk} />
        <MoneyField variant="cell" size="xs" blankZero value={entry.right} onChange={(v) => set({ right: v })} aria-label={cols.right} placeholder={cols.right} error={errFor('amount')} />
        <span className={classes.num}>
          <Group gap={4} justify="flex-end" wrap="nowrap">
            {onCancel && (
              <Button size="compact-xs" variant="default" onClick={onCancel}>
                Cancel
              </Button>
            )}
            <Button size="compact-xs" onClick={() => void record()} loading={busy} disabled={!editing && isBlankEntry(entry)}>
              {editing ? 'Save' : 'Record'}
            </Button>
          </Group>
        </span>
      </div>
      <div className={classes.r2}>
        <span />
        <span className={classes.type}>{account.account_type === 'Bank' ? (entry.right ? 'DEP' : 'CHK') : ''}</span>
        <AccountQuickFill variant="cell" size="xs" value={entry.account_id} onChange={(id) => set({ account_id: id })} aria-label="Account" placeholder="Account" error={errFor('account')} />
        <TextInput
          variant="unstyled"
          size="xs"
          className={classes.memo}
          classNames={{ input: 'lb-cell-text' }}
          value={entry.memo}
          onChange={(e) => set({ memo: e.currentTarget.value })}
          aria-label="Memo"
          placeholder={error ? error.message : 'Memo'}
          styles={error ? { input: { color: 'var(--mantine-color-error)' } } : undefined}
        />
      </div>
    </div>
  );
}

function MobileRegister({ account, lines, onOpen, onToggleCleared }: RegisterGridProps) {
  const scrollEl = useRef<HTMLDivElement>(null);
  // Newest first on a phone: what you just did is what you look for.
  const rows = lines;
  const n = rows.length;
  const virtualizer = useVirtualizer({ count: n, getScrollElement: () => scrollEl.current, estimateSize: () => 68, overscan: 8 });
  return (
    <div ref={scrollEl} className={classes.mobileScroll} data-testid="register-list">
      <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
        {virtualizer.getVirtualItems().map((vi) => {
          const l = rows[n - 1 - vi.index]! as Line;
          const amount = naturalBalance(account.account_type, l.amount);
          return (
            <div key={l.txn_id} className={classes.card} style={{ position: 'absolute', top: 0, left: 0, right: 0, height: 68, transform: `translateY(${vi.start}px)` }}>
              <UnstyledButton className={classes.cardMain} onClick={() => onOpen(l)}>
                <Group justify="space-between" wrap="nowrap" gap="xs">
                  <Text fw={600} size="sm" truncate>
                    {l.payee ?? TXN_REGISTER_CODE[l.txn_type]}
                  </Text>
                  <Text fw={600} size="sm" c={amount < 0 ? undefined : 'teal'} className={classes.mnum}>
                    {amount > 0 ? '+' : ''}
                    {formatMoney(amount)}
                  </Text>
                </Group>
                <Group justify="space-between" wrap="nowrap" gap="xs">
                  <Text size="xs" c="dimmed" truncate>
                    {formatDateShort(l.date)} · {TXN_REGISTER_CODE[l.txn_type]}
                    {l.number ? ` ${l.number}` : ''} · {l.is_split ? 'Split' : l.other_account}
                    {l.downloaded ? ' · ⚡' : ''}
                  </Text>
                  <Text size="xs" c="dimmed" className={classes.mnum}>
                    {formatMoney(naturalBalance(account.account_type, l.balance))}
                  </Text>
                </Group>
              </UnstyledButton>
              <ActionIcon
                variant="subtle"
                color={l.cleared === 'Cleared' ? 'teal' : 'gray'}
                size="lg"
                aria-label={l.cleared === 'Cleared' ? 'Cleared (tap to unclear)' : 'Not cleared (tap to clear)'}
                onClick={() => onToggleCleared(l)}
              >
                {l.cleared === 'Cleared' ? <IconCircleCheck size={22} /> : <IconCircleDashed size={22} />}
              </ActionIcon>
            </div>
          );
        })}
      </div>
    </div>
  );
}
