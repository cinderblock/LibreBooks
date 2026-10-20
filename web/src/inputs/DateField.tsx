// DateField: QuickBooks-style date entry.
//
// - Type it how you like: `10/19`, `10-19-26`, `101926`, `19` (this month), `t`.
// - QBD's keys work on the current date: `+`/`-` a day, `T` today, `W`/`K` week
//   start/end, `M`/`H` month start/end, `Y`/`R` year start/end, `[`/`]` a week.
//   `+`/`-` only act as keys when the field holds a whole date (fresh focus, or
//   all selected); mid-typing they're separators, so `10-19-26` still works.
// - Alt+↓ (or the calendar button, or a tap on a phone) opens a calendar.
// - Value in and out is an ISO date string (`2026-10-19`) or null.

import { ActionIcon, Popover, TextInput } from '@mantine/core';
import { DatePicker } from '@mantine/dates';
import { useMergedRef } from '@mantine/hooks';
import { IconCalendar } from '@tabler/icons-react';
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode, type Ref } from 'react';
import { dateKey, formatDate, parseTypedDate, todayIso, type IsoDate } from '~/lib/dates';
import classes from './cell.module.css';

export interface DateFieldProps {
  value: IsoDate | null;
  onChange(value: IsoDate | null): void;
  label?: ReactNode;
  error?: ReactNode;
  /** Shown under the field without blocking (QBD's "date is far in the past" warning). */
  warning?: ReactNode;
  required?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  size?: 'xs' | 'sm' | 'md';
  variant?: 'default' | 'cell';
  'aria-label'?: string;
  inputRef?: Ref<HTMLInputElement>;
  onKeyDown?(e: KeyboardEvent<HTMLInputElement>): void;
  inputProps?: Record<string, string | number | boolean | undefined>;
  /** Override "today" (tests, or a company's server date). */
  today?: IsoDate;
  className?: string;
}

const KEY_LETTERS = new Set(['t', 'w', 'k', 'm', 'h', 'y', 'r', '[', ']']);
const STEP_KEYS = new Set(['+', '=', '-', '_']);

export function DateField(props: DateFieldProps) {
  const { value, onChange, size = 'sm', variant = 'default' } = props;
  const inputEl = useRef<HTMLInputElement>(null);
  const ref = useMergedRef(inputEl, props.inputRef ?? null);
  const [text, setText] = useState(formatDate(value));
  const [invalid, setInvalid] = useState(false);
  const [calendar, setCalendar] = useState(false);
  const editing = useRef(false);

  useEffect(() => {
    if (!editing.current) {
      setText(formatDate(value));
      setInvalid(false);
    }
  }, [value]);

  const today = props.today ?? todayIso();

  /** The date the keys act on: what's typed if it parses, else the value, else today. */
  const current = (): IsoDate => parseTypedDate(text, today) ?? value ?? today;

  const set = (iso: IsoDate) => {
    setText(formatDate(iso));
    setInvalid(false);
    editing.current = false;
    if (iso !== value) onChange(iso);
    requestAnimationFrame(() => inputEl.current?.select());
  };

  const commit = () => {
    editing.current = false;
    const t = text.trim();
    if (t === '') {
      setInvalid(false);
      if (value !== null) onChange(null);
      return;
    }
    const iso = parseTypedDate(t, today);
    if (!iso) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    setText(formatDate(iso));
    if (iso !== value) onChange(iso);
  };

  const wholeDateSelected = (el: HTMLInputElement) =>
    el.value === '' || (el.selectionStart === 0 && el.selectionEnd === el.value.length) || el.value === formatDate(value);

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' && e.altKey) {
      e.preventDefault();
      setCalendar(true);
      return;
    }
    if (!e.ctrlKey && !e.metaKey && !e.altKey) {
      const k = e.key.toLowerCase();
      if (KEY_LETTERS.has(k) || (STEP_KEYS.has(k) && wholeDateSelected(e.currentTarget))) {
        const next = dateKey(current(), today, k);
        if (next) {
          e.preventDefault();
          set(next);
          return;
        }
      }
    }
    if (e.key === 'Enter') commit();
    if (e.key === 'Escape' && text !== formatDate(value)) {
      // First Esc undoes the typing; the next one reaches the window.
      e.preventDefault();
      e.stopPropagation();
      editing.current = false;
      setText(formatDate(value));
      setInvalid(false);
      return;
    }
    props.onKeyDown?.(e);
  };

  const error = props.error ?? (invalid ? 'Not a date (try 10/19 or 101926)' : undefined);

  return (
    <Popover opened={calendar} onChange={setCalendar} position="bottom-start" trapFocus returnFocus withinPortal shadow="md">
      <Popover.Target>
        <TextInput
          ref={ref}
          label={props.label}
          error={error}
          description={variant === 'cell' ? undefined : props.warning}
          classNames={variant === 'cell' ? { input: classes.cellInput } : { description: 'lb-warning' }}
          required={props.required}
          disabled={props.disabled}
          autoFocus={props.autoFocus}
          size={size}
          className={props.className}
          value={text}
          placeholder="MM/DD/YYYY"
          inputMode="decimal"
          enterKeyHint="next"
          autoComplete="off"
          aria-label={props['aria-label']}
          aria-invalid={!!error}
          title={props.warning && variant === 'cell' ? String(props.warning) : undefined}
          onChange={(e) => {
            editing.current = true;
            setText(e.currentTarget.value);
            setInvalid(false);
          }}
          onFocus={(e) => {
            const el = e.currentTarget;
            requestAnimationFrame(() => el.select());
          }}
          onBlur={commit}
          onKeyDown={onKeyDown}
          rightSectionPointerEvents="all"
          rightSection={
            variant === 'cell' ? null : (
              <ActionIcon
                variant="subtle"
                color="gray"
                size="sm"
                tabIndex={-1}
                aria-label="Open calendar"
                onClick={() => setCalendar((o) => !o)}
              >
                <IconCalendar size={16} />
              </ActionIcon>
            )
          }
          {...props.inputProps}
        />
      </Popover.Target>
      <Popover.Dropdown>
        <DatePicker
          value={parseTypedDate(text, today) ?? value}
          defaultDate={parseTypedDate(text, today) ?? value ?? today}
          onChange={(v) => {
            setCalendar(false);
            if (v) set(v.slice(0, 10));
            inputEl.current?.focus();
          }}
          firstDayOfWeek={0}
          highlightToday
        />
      </Popover.Dropdown>
    </Popover>
  );
}
