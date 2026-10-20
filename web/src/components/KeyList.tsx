// KeyList: the keyboard-driven, virtualized list behind QBD's list windows (Chart
// of Accounts, centres, Item List). ↑/↓/PgUp/PgDn/Home/End move the selection,
// Enter opens, typing a letter jumps to the next entry starting with it (as QBD
// lists do). On phones each entry is a tappable row instead.

import { Text, UnstyledButton } from '@mantine/core';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useRef, type CSSProperties, type KeyboardEvent, type ReactNode } from 'react';
import classes from '~/screens/screens.module.css';

export interface KeyListColumn<T> {
  key: string;
  header: string;
  /** CSS grid track: `minmax(200px, 2fr)`, `120px`. */
  width: string;
  align?: 'left' | 'right';
  render(item: T): ReactNode;
}

export interface KeyListProps<T> {
  items: readonly T[];
  rowKey(item: T): string;
  columns: KeyListColumn<T>[];
  selected: number;
  onSelect(index: number): void;
  onOpen(item: T): void;
  /** Text used for type-to-jump. */
  jumpText?(item: T): string;
  rowAttrs?(item: T): Record<string, boolean | undefined>;
  label: string;
  empty?: ReactNode;
  mobile?: boolean;
  /** Phone row content (defaults to the first two columns). */
  renderMobile?(item: T): ReactNode;
  autoFocus?: boolean;
}

export function KeyList<T>(props: KeyListProps<T>) {
  const { items, selected, onSelect, columns, mobile } = props;
  const scrollEl = useRef<HTMLDivElement>(null);
  const rowH = mobile ? 58 : 30;
  const virtualizer = useVirtualizer({ count: items.length, getScrollElement: () => scrollEl.current, estimateSize: () => rowH, overscan: 12 });
  const template = { gridTemplateColumns: columns.map((c) => c.width).join(' ') } as CSSProperties;

  useEffect(() => {
    if (items.length && selected >= 0) virtualizer.scrollToIndex(Math.min(selected, items.length - 1), { align: 'auto' });
  }, [selected, items.length]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (props.autoFocus) scrollEl.current?.focus({ preventScroll: true });
  }, [props.autoFocus]);

  const move = (i: number) => onSelect(Math.max(0, Math.min(items.length - 1, i)));
  const page = Math.max(1, Math.floor((scrollEl.current?.clientHeight ?? 300) / rowH) - 1);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        move(selected + 1);
        return;
      case 'ArrowUp':
        e.preventDefault();
        move(selected - 1);
        return;
      case 'PageDown':
        e.preventDefault();
        move(selected + page);
        return;
      case 'PageUp':
        e.preventDefault();
        move(selected - page);
        return;
      case 'Home':
        e.preventDefault();
        move(0);
        return;
      case 'End':
        e.preventDefault();
        move(items.length - 1);
        return;
      case 'Enter': {
        const item = items[selected];
        if (item) {
          e.preventDefault();
          props.onOpen(item);
        }
        return;
      }
    }
    if (props.jumpText && e.key.length === 1 && /\S/.test(e.key)) {
      // Type-to-jump: the next entry whose name starts with the letter.
      const ch = e.key.toLowerCase();
      for (let k = 1; k <= items.length; k++) {
        const i = (selected + k) % items.length;
        if (props.jumpText(items[i]!).toLowerCase().startsWith(ch)) {
          e.preventDefault();
          move(i);
          return;
        }
      }
    }
  };

  return (
    <div
      ref={scrollEl}
      className={classes.list}
      tabIndex={0}
      role="grid"
      aria-label={props.label}
      aria-rowcount={items.length}
      onKeyDown={onKeyDown}
    >
      {!mobile && (
        <div className={classes.listHead} style={template} role="row">
          {columns.map((c) => (
            <div key={c.key} role="columnheader" className={c.align === 'right' ? classes.right : undefined}>
              {c.header}
            </div>
          ))}
        </div>
      )}
      {items.length === 0 && (
        <Text c="dimmed" size="sm" p="md">
          {props.empty ?? 'Nothing to show.'}
        </Text>
      )}
      <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
        {virtualizer.getVirtualItems().map((vi) => {
          const item = items[vi.index]!;
          const style: CSSProperties = { position: 'absolute', top: 0, left: 0, right: 0, height: rowH, transform: `translateY(${vi.start}px)` };
          if (mobile) {
            return (
              <UnstyledButton
                key={props.rowKey(item)}
                className={classes.mobileRow}
                style={style}
                onClick={() => {
                  onSelect(vi.index);
                  props.onOpen(item);
                }}
                {...props.rowAttrs?.(item)}
              >
                {props.renderMobile ? props.renderMobile(item) : columns.slice(0, 2).map((c) => <span key={c.key}>{c.render(item)}</span>)}
              </UnstyledButton>
            );
          }
          return (
            <div
              key={props.rowKey(item)}
              role="row"
              aria-selected={vi.index === selected}
              className={classes.listRow}
              data-selected={vi.index === selected || undefined}
              style={{ ...style, ...template }}
              onClick={() => {
                onSelect(vi.index);
                scrollEl.current?.focus({ preventScroll: true });
              }}
              onDoubleClick={() => props.onOpen(item)}
              {...props.rowAttrs?.(item)}
            >
              {columns.map((c) => (
                <div key={c.key} role="gridcell" className={c.align === 'right' ? classes.right : undefined}>
                  {c.render(item)}
                </div>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
