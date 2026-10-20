// LineGrid: the line-item grid of a transaction form (Write Checks' Expenses and
// Items tabs, invoice lines, journal lines).
//
// Keyboard (QBD-like, plus spreadsheet arrows):
// - Tab / Shift+Tab: next / previous cell (native order: cells are row-major).
// - ↑ / ↓: same column on the previous / next line (unless an open QuickFill
//   list is using the arrows).
// - ← / →: previous / next cell when the cell's whole content is selected (a
//   freshly entered cell); once you're editing, they move the caret.
// - Enter: configurable: next cell (default), next line, or save the form.
// - Ctrl+Insert / Ctrl+Delete (or ⌘⇧Enter / ⌘⇧⌫): insert / delete a line.
// There is always exactly one blank line at the bottom; typing in it adds a line.
//
// On phones the same rows render as one card per line, with an "Add line" button.

import { ActionIcon, Button, Card, Group, Stack, Text } from '@mantine/core';
import { IconPlus, IconTrash } from '@tabler/icons-react';
import { useCallback, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { useWindowEnabled } from '~/windows/WindowsProvider';
import { withTrailingBlank } from './gridRows';
import classes from './LineGrid.module.css';

export { withTrailingBlank };

export type EnterBehavior = 'next-cell' | 'next-row' | 'save';

export interface CellContext<R> {
  row: R;
  rowIndex: number;
  /** Replace this row (column logic like qty × rate lives in the caller's updater). */
  update(next: R | ((r: R) => R)): void;
  /** Spread onto the focusable element so the grid can navigate to it. */
  cellProps: { 'data-cell': string; 'aria-label': string };
  /** `cell` in the table, `default` (with a visible label) in a mobile card. */
  variant: 'cell' | 'default';
  label: string;
  error?: string;
  /** Call after a QuickFill choice made with Enter, to apply Enter behaviour. */
  onCommit(): void;
}

export interface GridColumn<R> {
  key: string;
  header: string;
  width?: number | string;
  align?: 'left' | 'right' | 'center';
  render(ctx: CellContext<R>): ReactNode;
  footer?: ReactNode;
  /** Phones: full-width field, half-width, or left out of the card. */
  mobile?: 'full' | 'half' | 'hidden';
}

export interface LineGridProps<R> {
  rows: R[];
  onChange(rows: R[]): void;
  columns: GridColumn<R>[];
  rowKey(r: R): string;
  isBlank(r: R): boolean;
  makeBlank(): R;
  enterBehavior?: EnterBehavior;
  onEnterSave?(): void;
  /** rowIndex → column key → message. */
  errors?: Record<number, Record<string, string>>;
  mobile?: boolean;
  label: string;
  /** Pad the table with empty-looking lines so it reads like a QBD form. */
  minVisibleRows?: number;
}

function cellOf(el: EventTarget | null): { r: number; c: number; el: HTMLElement } | null {
  const target = (el as HTMLElement | null)?.closest?.('[data-cell]') as HTMLElement | null;
  const v = target?.getAttribute('data-cell');
  if (!target || !v) return null;
  const [r, c] = v.split(':').map(Number) as [number, number];
  return { r, c, el: target };
}

export function LineGrid<R>(props: LineGridProps<R>) {
  const { rows, columns, enterBehavior = 'next-cell', mobile = false } = props;
  const container = useRef<HTMLDivElement>(null);
  const windowActive = useWindowEnabled();
  const [mobileBlankOpen, setMobileBlankOpen] = useState(false);

  const setRows = useCallback(
    (next: R[]) => props.onChange(withTrailingBlank(next, props.isBlank, props.makeBlank)),
    [props],
  );

  const focusCell = (r: number, c: number) => {
    requestAnimationFrame(() => {
      const el = container.current?.querySelector<HTMLElement>(`[data-cell="${r}:${c}"]`);
      el?.focus();
    });
  };

  const allCells = () => [...(container.current?.querySelectorAll<HTMLElement>('[data-cell]') ?? [])];

  const moveLinear = (from: HTMLElement, delta: 1 | -1) => {
    const cells = allCells();
    const i = cells.indexOf(from);
    cells[i + delta]?.focus();
  };

  const moveVertical = (r: number, c: number, delta: 1 | -1) => {
    const target = container.current?.querySelector<HTMLElement>(`[data-cell="${r + delta}:${c}"]`);
    target?.focus();
  };

  const enter = (from: HTMLElement, r: number, c: number) => {
    if (enterBehavior === 'save') {
      // Let the field commit its text (and React re-render) before saving.
      setTimeout(() => props.onEnterSave?.(), 0);
    } else if (enterBehavior === 'next-row') moveVertical(r, c, 1);
    else moveLinear(from, 1);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
    const cell = cellOf(e.target);
    if (!cell) return;
    const input = e.target instanceof HTMLInputElement ? e.target : null;
    const wholeSelected =
      !input || input.type === 'checkbox' || input.value === '' || (input.selectionStart === 0 && input.selectionEnd === input.value.length);
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp':
        e.preventDefault();
        moveVertical(cell.r, cell.c, e.key === 'ArrowDown' ? 1 : -1);
        break;
      case 'ArrowLeft':
      case 'ArrowRight':
        if (wholeSelected) {
          e.preventDefault();
          moveLinear(cell.el, e.key === 'ArrowRight' ? 1 : -1);
        }
        break;
      case 'Enter':
        e.preventDefault();
        enter(cell.el, cell.r, cell.c);
        break;
    }
  };

  const activeRow = (): number | null => {
    const cell = cellOf(document.activeElement);
    return cell && container.current?.contains(cell.el) ? cell.r : null;
  };
  const gridFocused = () => windowActive() && activeRow() !== null;

  const insertLine = (at: number) => {
    const next = [...rows];
    next.splice(at, 0, props.makeBlank());
    setRows(next);
  };
  const deleteLine = (at: number) => {
    const next = rows.filter((_, i) => i !== at);
    setRows(next);
  };

  useShortcut(
    'grid.insertLine',
    () => {
      const r = activeRow();
      if (r === null) return false;
      const c = cellOf(document.activeElement)?.c ?? 0;
      insertLine(r);
      focusCell(r, c);
    },
    { enabled: gridFocused },
  );
  useShortcut(
    'grid.deleteLine',
    () => {
      const r = activeRow();
      if (r === null) return false;
      const c = cellOf(document.activeElement)?.c ?? 0;
      deleteLine(r);
      focusCell(Math.min(r, rows.length - 2), c);
    },
    { enabled: gridFocused },
  );

  const ctx = (row: R, rowIndex: number, colIndex: number, col: GridColumn<R>, variant: 'cell' | 'default'): CellContext<R> => ({
    row,
    rowIndex,
    variant,
    label: col.header,
    error: props.errors?.[rowIndex]?.[col.key],
    update: (next) => {
      const updated = typeof next === 'function' ? (next as (r: R) => R)(row) : next;
      setRows(rows.map((r, i) => (i === rowIndex ? updated : r)));
    },
    cellProps: { 'data-cell': `${rowIndex}:${colIndex}`, 'aria-label': `${col.header}, line ${rowIndex + 1}` },
    onCommit: () => {
      const el = container.current?.querySelector<HTMLElement>(`[data-cell="${rowIndex}:${colIndex}"]`);
      if (el) enter(el, rowIndex, colIndex);
    },
  });

  if (mobile) {
    const lastBlank = rows.length > 0 && props.isBlank(rows[rows.length - 1]!) ? rows.length - 1 : -1;
    return (
      <Stack gap="sm" ref={container} onKeyDown={onKeyDown} aria-label={props.label}>
        {rows.map((row, i) => {
          if (i === lastBlank && !mobileBlankOpen && i > 0) return null;
          const rowErrors = props.errors?.[i];
          return (
            <Card key={props.rowKey(row)} withBorder padding="sm" className={classes.card} data-error={rowErrors ? true : undefined}>
              <Group justify="space-between" mb={4}>
                <Text size="xs" c="dimmed">
                  Line {i + 1}
                </Text>
                {i !== lastBlank && (
                  <ActionIcon variant="subtle" color="red" aria-label={`Delete line ${i + 1}`} onClick={() => deleteLine(i)}>
                    <IconTrash size={16} />
                  </ActionIcon>
                )}
              </Group>
              <div className={classes.cardFields}>
                {columns.map((col, c) =>
                  col.mobile === 'hidden' ? null : (
                    <div key={col.key} className={col.mobile === 'half' ? classes.half : classes.full}>
                      {col.render(ctx(row, i, c, col, 'default'))}
                    </div>
                  ),
                )}
              </div>
            </Card>
          );
        })}
        <Button
          variant="light"
          leftSection={<IconPlus size={16} />}
          onClick={() => {
            setMobileBlankOpen(true);
            focusCell(rows.length - 1, 0);
          }}
          disabled={mobileBlankOpen && lastBlank >= 0}
        >
          Add line
        </Button>
      </Stack>
    );
  }

  const padding = Math.max(0, (props.minVisibleRows ?? 0) - rows.length);
  return (
    <div className={classes.wrap} ref={container} onKeyDown={onKeyDown}>
      <table className={classes.table} aria-label={props.label}>
        <colgroup>
          {columns.map((c) => (
            <col key={c.key} style={{ width: c.width }} />
          ))}
          <col style={{ width: 28 }} />
        </colgroup>
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={c.key} style={{ textAlign: c.align ?? 'left' }}>
                {c.header}
              </th>
            ))}
            <th aria-label="Line actions" />
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={props.rowKey(row)} data-error={props.errors?.[i] ? true : undefined}>
              {columns.map((col, c) => (
                <td key={col.key} style={{ textAlign: col.align ?? 'left' }} data-invalid={props.errors?.[i]?.[col.key] ? true : undefined}>
                  {col.render(ctx(row, i, c, col, 'cell'))}
                </td>
              ))}
              <td className={classes.actions}>
                {!props.isBlank(row) && (
                  <ActionIcon
                    variant="subtle"
                    color="gray"
                    size="sm"
                    tabIndex={-1}
                    aria-label={`Delete line ${i + 1}`}
                    title="Delete line (Ctrl+Delete)"
                    onClick={() => deleteLine(i)}
                  >
                    <IconTrash size={14} />
                  </ActionIcon>
                )}
              </td>
            </tr>
          ))}
          {Array.from({ length: padding }, (_, i) => (
            <tr key={`pad-${i}`} className={classes.pad} aria-hidden>
              {columns.map((c) => (
                <td key={c.key} />
              ))}
              <td />
            </tr>
          ))}
        </tbody>
        {columns.some((c) => c.footer !== undefined) && (
          <tfoot>
            <tr>
              {columns.map((c) => (
                <td key={c.key} style={{ textAlign: c.align ?? 'left' }}>
                  {c.footer}
                </td>
              ))}
              <td />
            </tr>
          </tfoot>
        )}
      </table>
    </div>
  );
}
