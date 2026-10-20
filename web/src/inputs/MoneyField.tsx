// MoneyField: an amount in cents, with QuickBooks' built-in calculator.
//
// - Accepts `1,234.56`, `$5`, `-12`, `(12.00)`.
// - Arithmetic: type `12*3+4` and press `=`, Enter or Tab (or just leave) to get
//   40.00. A "tape" under the field shows the running result while you type.
// - On a filled field (fresh focus, all selected), typing `+`, `-`, `*` or `/`
//   starts arithmetic on the current amount instead of replacing it: `+5` on
//   100.00 makes 105.00.
// - Esc undoes what you typed; formatting (`1,234.56`) is applied on leaving.
// - Exact: the calculator works in rationals and rounds once, half away from zero.

import { Popover, TextInput } from '@mantine/core';
import { useMergedRef } from '@mantine/hooks';
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode, type Ref } from 'react';
import { evaluate, isExpression } from '~/lib/calc';
import { formatMoney, MONEY_ERROR_TEXT, parseMoney, type Cents } from '~/lib/money';
import classes from './cell.module.css';

export interface MoneyFieldProps {
  value: Cents | null;
  onChange(value: Cents | null): void;
  label?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  placeholder?: string;
  size?: 'xs' | 'sm' | 'md';
  variant?: 'default' | 'cell';
  /** Show zero as an empty field (grid amount cells). */
  blankZero?: boolean;
  allowNegative?: boolean;
  'aria-label'?: string;
  inputRef?: Ref<HTMLInputElement>;
  onKeyDown?(e: KeyboardEvent<HTMLInputElement>): void;
  inputProps?: Record<string, string | number | boolean | undefined>;
  className?: string;
  leftSection?: ReactNode;
}

const OPERATORS = new Set(['+', '-', '*', '/']);

export function MoneyField(props: MoneyFieldProps) {
  const { value, onChange, size = 'sm', variant = 'default', blankZero = false, allowNegative = true } = props;
  const inputEl = useRef<HTMLInputElement>(null);
  const ref = useMergedRef(inputEl, props.inputRef ?? null);
  const display = (v: Cents | null) => (v === null || (blankZero && v === 0) ? '' : formatMoney(v));
  const [text, setText] = useState(display(value));
  const [error, setError] = useState<string | null>(null);
  const editing = useRef(false);

  useEffect(() => {
    if (!editing.current) {
      setText(display(value));
      setError(null);
    }
  }, [value]); // eslint-disable-line react-hooks/exhaustive-deps

  const expr = isExpression(text);
  const preview = expr ? evaluate(text) : null;

  /** Turn the text into cents (evaluating arithmetic); false if it isn't an amount. */
  const commit = (): boolean => {
    editing.current = false;
    const t = text.trim();
    if (t === '' || t === '=') {
      setError(null);
      setText('');
      if (value !== null) onChange(null);
      return true;
    }
    let cents: Cents;
    if (isExpression(t) || t.endsWith('=')) {
      const r = evaluate(t);
      if (!r.ok) {
        setError(r.error);
        return false;
      }
      cents = r.cents;
    } else {
      const r = parseMoney(t);
      if (!r.ok) {
        setError(MONEY_ERROR_TEXT[r.error]);
        return false;
      }
      cents = r.cents;
    }
    if (!allowNegative && cents < 0) {
      setError('The amount must not be negative');
      return false;
    }
    setError(null);
    setText(display(cents));
    if (cents !== value) onChange(cents);
    return true;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const el = e.currentTarget;
    const allSelected = el.value !== '' && el.selectionStart === 0 && el.selectionEnd === el.value.length;
    if (!e.ctrlKey && !e.metaKey && !e.altKey) {
      if (OPERATORS.has(e.key) && allSelected && value !== null && value !== 0) {
        // Arithmetic on the current amount (QBD): 100.00 then "+5" → 105.00.
        e.preventDefault();
        editing.current = true;
        const start = formatMoney(value, { grouping: false });
        setText(`${start}${e.key}`);
        requestAnimationFrame(() => el.setSelectionRange(el.value.length, el.value.length));
        return;
      }
      if (e.key === '=') {
        e.preventDefault();
        if (commit()) requestAnimationFrame(() => el.select());
        return;
      }
    }
    if (e.key === 'Enter' || e.key === 'Tab') commit();
    if (e.key === 'Escape' && text !== display(value)) {
      e.preventDefault();
      e.stopPropagation();
      editing.current = false;
      setText(display(value));
      setError(null);
      return;
    }
    props.onKeyDown?.(e);
  };

  const shownError = props.error ?? error ?? undefined;
  const tape =
    preview && editing.current ? (
      <div className={classes.tape} data-error={!preview.ok || undefined}>
        {text.replace(/=$/, '')} = {preview.ok ? formatMoney(preview.cents) : preview.error}
      </div>
    ) : null;

  return (
    <Popover opened={!!tape} position="bottom-end" withinPortal trapFocus={false} returnFocus={false} offset={2} shadow="sm">
      <Popover.Target>
        <TextInput
          ref={ref}
          label={props.label}
          error={variant === 'cell' ? undefined : shownError}
          required={props.required}
          disabled={props.disabled}
          autoFocus={props.autoFocus}
          placeholder={props.placeholder}
          size={size}
          className={props.className}
          leftSection={props.leftSection}
          classNames={{ input: variant === 'cell' ? `${classes.cellInput} ${classes.amount}` : classes.amount }}
          value={text}
          inputMode="decimal"
          enterKeyHint="next"
          autoComplete="off"
          aria-label={props['aria-label']}
          aria-invalid={!!shownError}
          title={variant === 'cell' && shownError ? String(shownError) : undefined}
          onChange={(e) => {
            editing.current = true;
            setText(e.currentTarget.value);
            setError(null);
          }}
          onFocus={(e) => {
            const el = e.currentTarget;
            requestAnimationFrame(() => el.select());
          }}
          onBlur={() => {
            commit();
          }}
          onKeyDown={onKeyDown}
          {...props.inputProps}
        />
      </Popover.Target>
      <Popover.Dropdown p={0}>{tape}</Popover.Dropdown>
    </Popover>
  );
}
