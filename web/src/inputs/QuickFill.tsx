// QuickFill: QuickBooks' type-ahead for accounts, names, items and classes.
//
// - Typing completes inline when the best match starts with what was typed (the
//   completed part is selected, so the next keystroke replaces it), and the list
//   highlights the best match at any colon level (`Mat` → `…:Materials`).
// - Fully keyboard driven: ↓/↑/PgUp/PgDn move, Enter/Tab choose, Esc closes (and
//   only then lets Esc reach the window), Alt+↓ or Ctrl+L opens the whole list.
// - "<Add New>" sits at the top of the list; typing a name that isn't in the list
//   and leaving the field offers to create it (QBD's "not in the list" prompt).
// - The list is virtualized, so thousands of entries stay instant.
// - Inactive entries are hidden unless asked for (or already chosen).

import { Popover, TextInput, Text, UnstyledButton } from '@mantine/core';
import { useMergedRef } from '@mantine/hooks';
import { IconChevronDown } from '@tabler/icons-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from 'react';
import { exactIndex, inlineCompletion, prepare, rankOf, Rank, search } from '~/lib/quickfill';
import { parentPath } from '~/lib/tree';
import cell from './cell.module.css';
import classes from './QuickFill.module.css';

export interface QuickFillOption {
  id: string;
  /** `Cost of Goods Sold:Materials` (what the field shows once chosen). */
  fullName: string;
  /** Leaf name: `Materials`. */
  name: string;
  depth: number;
  inactive?: boolean;
  /** Extra prefix-searchable keys (account number, company name). */
  keys?: string[];
  /** Right-hand columns (type, balance…), aligned under `columnHeaders`. */
  columns?: ReactNode[];
}

export interface QuickFillProps {
  options: readonly QuickFillOption[];
  value: string | null;
  onChange(id: string | null): void;
  /** Offer "<Add New>"; resolve with the created id (or null if cancelled). */
  onAddNew?(text: string): Promise<string | null>;
  label?: ReactNode;
  placeholder?: string;
  error?: ReactNode;
  required?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  'aria-label'?: string;
  columnHeaders?: string[];
  showInactive?: boolean;
  size?: 'xs' | 'sm' | 'md';
  /** Grid cells: no border, fills the cell. */
  variant?: 'default' | 'cell';
  inputRef?: Ref<HTMLInputElement>;
  /** Runs after QuickFill's own handling (if it didn't consume the key). */
  onKeyDown?(e: KeyboardEvent<HTMLInputElement>): void;
  /** After a choice made with Enter (grids move on). */
  onCommit?(): void;
  /** Extra attributes for the input element (data-cell for grids). */
  inputProps?: Record<string, string | number | boolean | undefined>;
  dropdownMinWidth?: number;
  className?: string;
  style?: CSSProperties;
}

type Row = { type: 'add' } | { type: 'option'; index: number };

const coarsePointer = () => typeof window !== 'undefined' && window.matchMedia?.('(pointer: coarse)').matches;

export function QuickFill(props: QuickFillProps) {
  const {
    options,
    value,
    onChange,
    onAddNew,
    showInactive = false,
    size = 'sm',
    variant = 'default',
    columnHeaders,
    dropdownMinWidth = 380,
  } = props;
  const listId = useId();
  const inputEl = useRef<HTMLInputElement>(null);
  const mergedRef = useMergedRef(inputEl, props.inputRef ?? null);
  const scrollEl = useRef<HTMLDivElement>(null);

  const prepared = useMemo(() => prepare(options.map((o) => ({ fullName: o.fullName, keys: o.keys }))), [options]);
  const selected = useMemo(() => options.find((o) => o.id === value) ?? null, [options, value]);

  const [text, setText] = useState(selected?.fullName ?? '');
  const [query, setQuery] = useState<string | null>(null); // null: not typing
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const pendingSelection = useRef<[number, number] | null>(null);
  const focused = useRef(false);
  // Tab settles, then the blur that follows must not settle again (it would see
  // stale state and could offer Quick Add twice). Reset by typing or refocusing.
  const settled = useRef(false);

  // Follow the value when it changes from outside (and we're not mid-edit).
  useEffect(() => {
    if (!focused.current || query === null) setText(selected?.fullName ?? '');
  }, [selected]); // eslint-disable-line react-hooks/exhaustive-deps

  const visible = useCallback(
    (i: number) => showInactive || !options[i]!.inactive || options[i]!.id === value,
    [options, showInactive, value],
  );

  const rows: Row[] = useMemo(() => {
    const matches = search(prepared, query ?? '').filter(visible);
    const out: Row[] = matches.map((index) => ({ type: 'option', index }));
    if (onAddNew) out.unshift({ type: 'add' });
    return out;
  }, [prepared, query, visible, onAddNew]);

  const filtering = !!query;
  const rowHeight = coarsePointer() ? 40 : 28;
  const virtualizer = useVirtualizer({
    count: open ? rows.length : 0,
    getScrollElement: () => scrollEl.current,
    estimateSize: () => rowHeight,
    overscan: 12,
  });

  useEffect(() => {
    if (open && rows.length) virtualizer.scrollToIndex(Math.min(active, rows.length - 1), { align: 'auto' });
  }, [active, open, rows.length]); // eslint-disable-line react-hooks/exhaustive-deps

  // Apply the inline-completion selection after React has written the text.
  useLayoutEffect(() => {
    const sel = pendingSelection.current;
    if (sel && inputEl.current) {
      inputEl.current.setSelectionRange(sel[0], sel[1]);
      pendingSelection.current = null;
    }
  });

  const firstOptionRow = onAddNew ? 1 : 0;
  const rowIndexOf = (id: string | null) => {
    const r = rows.findIndex((row) => row.type === 'option' && options[row.index]!.id === id);
    return r < 0 ? firstOptionRow : r;
  };

  const openList = (all: boolean) => {
    if (all) setQuery('');
    setOpen(true);
    setActive(all ? rowIndexOfAll(value) : firstOptionRow);
  };
  // Index in the unfiltered list (query '' → every visible option).
  const rowIndexOfAll = (id: string | null) => {
    const all = prepared.map((_, i) => i).filter(visible);
    const i = all.findIndex((x) => options[x]!.id === id);
    return (i < 0 ? 0 : i) + firstOptionRow;
  };

  const close = () => {
    setOpen(false);
    setQuery(null);
  };

  const choose = (id: string | null) => {
    settled.current = true;
    const opt = options.find((o) => o.id === id) ?? null;
    setText(opt?.fullName ?? '');
    close();
    if (id !== value) onChange(id);
  };

  const addNew = async (seed: string) => {
    close();
    if (!onAddNew) return;
    const id = await onAddNew(seed.trim());
    if (id) {
      onChange(id);
      // The new record arrives with the next list refresh; show the typed name meanwhile.
      setText(seed.trim());
    } else setText(selected?.fullName ?? '');
    inputEl.current?.focus();
  };

  const chooseRow = (r: number) => {
    const row = rows[r];
    if (!row) return;
    if (row.type === 'add') void addNew(query ?? '');
    else choose(options[row.index]!.id);
  };

  /** Leaving the field (Tab, blur): settle what was typed into a value. */
  const settle = () => {
    if (settled.current) return;
    settled.current = true;
    if (open && query !== null && rows[active]?.type === 'option') {
      chooseRow(active);
      return;
    }
    const t = text.trim();
    if (t === '') {
      choose(null);
      return;
    }
    if (selected && t === selected.fullName) {
      close();
      return;
    }
    const exact = exactIndex(prepared, t);
    if (exact >= 0) {
      choose(options[exact]!.id);
      return;
    }
    // Not in the list: QBD offers Quick Add.
    if (onAddNew) void addNew(t);
    else {
      setText(selected?.fullName ?? '');
      close();
    }
  };

  const onInput = (e: ChangeEvent<HTMLInputElement>) => {
    const typed = e.target.value;
    settled.current = false;
    const inserting = (e.nativeEvent as InputEvent).inputType?.startsWith('insert') ?? true;
    setQuery(typed);
    setOpen(true);
    const matches = search(prepared, typed).filter(visible);
    const best = matches[0];
    setActive(best === undefined ? 0 : firstOptionRow);
    const atEnd = e.target.selectionStart === typed.length;
    if (best !== undefined && inserting && atEnd && rankOf(prepared[best]!, typed.toLowerCase()) === Rank.FullPrefix) {
      const completed = inlineCompletion(typed, options[best]!.fullName);
      if (completed) {
        setText(completed);
        pendingSelection.current = [typed.length, completed.length];
        return;
      }
    }
    setText(typed);
  };

  const highlight = (r: number) => {
    const clamped = Math.max(0, Math.min(rows.length - 1, r));
    setActive(clamped);
    const row = rows[clamped];
    if (row?.type === 'option') {
      // Show the highlighted entry in the field, as QBD does while arrowing.
      setText(options[row.index]!.fullName);
      pendingSelection.current = [0, options[row.index]!.fullName.length];
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const k = e.key;
    if ((k === 'ArrowDown' && e.altKey) || (k.toLowerCase() === 'l' && (e.ctrlKey || e.metaKey) && !e.shiftKey)) {
      e.preventDefault();
      openList(true);
      return;
    }
    if (open) {
      if (k === 'ArrowDown' || k === 'ArrowUp' || k === 'PageDown' || k === 'PageUp') {
        e.preventDefault();
        const step = k === 'ArrowDown' ? 1 : k === 'ArrowUp' ? -1 : k === 'PageDown' ? 8 : -8;
        highlight(active + step);
        return;
      }
      if (k === 'Enter') {
        e.preventDefault();
        chooseRow(active);
        props.onCommit?.();
        return;
      }
      if (k === 'Escape') {
        // Close the list only; the window keeps its Esc for next time.
        e.preventDefault();
        e.stopPropagation();
        setText(selected?.fullName ?? '');
        close();
        return;
      }
      if (k === 'Tab') {
        settle(); // focus moves on naturally
        props.onKeyDown?.(e);
        return;
      }
    } else if (k === 'ArrowDown' && query !== null) {
      // Typed something, list was closed with Esc: arrows reopen it.
      e.preventDefault();
      openList(false);
      return;
    } else if (k === 'Tab' || k === 'Enter') {
      if (query !== null) settle();
    }
    props.onKeyDown?.(e);
  };

  const onFocus = () => {
    focused.current = true;
    settled.current = false;
    requestAnimationFrame(() => inputEl.current?.select());
    if (coarsePointer() && !props.disabled) openList(true);
  };

  const onBlur = () => {
    focused.current = false;
    if (query !== null || text !== (selected?.fullName ?? '')) settle();
    else close();
  };

  const activeId = open && rows[active] ? `${listId}-opt-${active}` : undefined;

  const input = (
    <TextInput
      ref={mergedRef}
      label={props.label}
      placeholder={props.placeholder}
      error={props.error}
      required={props.required}
      disabled={props.disabled}
      autoFocus={props.autoFocus}
      size={size}
      className={props.className}
      style={props.style}
      classNames={variant === 'cell' ? { input: cell.cellInput } : undefined}
      value={text}
      onChange={onInput}
      onKeyDown={onKeyDown}
      onFocus={onFocus}
      onBlur={onBlur}
      autoComplete="off"
      autoCorrect="off"
      spellCheck={false}
      inputMode="search"
      enterKeyHint="next"
      role="combobox"
      aria-label={props['aria-label']}
      aria-expanded={open}
      aria-controls={listId}
      aria-autocomplete="both"
      aria-activedescendant={activeId}
      rightSectionPointerEvents="all"
      rightSection={
        variant === 'cell' ? null : (
          <UnstyledButton
            tabIndex={-1}
            aria-label="Show list"
            className={classes.chevron}
            onMouseDown={(e) => {
              e.preventDefault();
              if (open) close();
              else {
                inputEl.current?.focus();
                openList(true);
              }
            }}
          >
            <IconChevronDown size={14} />
          </UnstyledButton>
        )
      }
      {...props.inputProps}
    />
  );

  return (
    <Popover
      opened={open && !props.disabled}
      position="bottom-start"
      width="target"
      trapFocus={false}
      returnFocus={false}
      withinPortal
      shadow="md"
      offset={2}
      middlewares={{ flip: true, shift: true }}
      // Never wider than a phone: the list opens on tap at 375px and must not push the page sideways.
      styles={{ dropdown: { padding: 0, minWidth: `min(${dropdownMinWidth}px, calc(100vw - 16px))` } }}
    >
      <Popover.Target>{input}</Popover.Target>
      <Popover.Dropdown
        // Keep focus in the input while clicking the list.
        onMouseDown={(e) => e.preventDefault()}
      >
        {columnHeaders && (
          <div className={classes.header}>
            <span>Name</span>
            {columnHeaders.map((h) => (
              <span key={h} className={classes.col}>
                {h}
              </span>
            ))}
          </div>
        )}
        <div ref={scrollEl} className={classes.scroll} id={listId} role="listbox" style={{ maxHeight: coarsePointer() ? '40vh' : 320 }}>
          {rows.length === (onAddNew ? 1 : 0) && (
            <Text size="sm" c="dimmed" p="xs">
              Nothing matches "{query}".
            </Text>
          )}
          <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
            {virtualizer.getVirtualItems().map((vi) => {
              const row = rows[vi.index]!;
              const isActive = vi.index === active;
              const common = {
                id: `${listId}-opt-${vi.index}`,
                role: 'option' as const,
                'aria-selected': isActive,
                className: classes.row,
                'data-active': isActive || undefined,
                style: { position: 'absolute' as const, top: 0, left: 0, right: 0, height: rowHeight, transform: `translateY(${vi.start}px)` },
                onMouseEnter: () => setActive(vi.index),
                onClick: () => {
                  chooseRow(vi.index);
                  props.onCommit?.();
                },
              };
              if (row.type === 'add') {
                return (
                  <div key="add" {...common} data-add>
                    <span className={classes.add}>&lt;Add New&gt;</span>
                  </div>
                );
              }
              const o = options[row.index]!;
              const path = parentPath(o.fullName);
              return (
                <div key={o.id} {...common} data-inactive={o.inactive || undefined} data-selected={o.id === value || undefined}>
                  <span className={classes.name} style={{ paddingLeft: filtering ? 0 : o.depth * 14 }}>
                    {o.name}
                    {filtering && path && <span className={classes.path}> — {path}</span>}
                    {o.inactive && <span className={classes.path}> (inactive)</span>}
                  </span>
                  {o.columns?.map((c, i) => (
                    <span key={i} className={classes.col}>
                      {c}
                    </span>
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      </Popover.Dropdown>
    </Popover>
  );
}
