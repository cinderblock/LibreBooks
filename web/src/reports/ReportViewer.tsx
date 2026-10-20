// ReportViewer: renders any `Report` (columns, outlined rows, totals).
//
// - Sections collapse/expand (click the triangle, or ←/→ on the selected row);
//   a collapsed section shows its subtotal on its own line, as QBD does.
// - QuickZoom: double-click (tap on a phone, or Enter) a row that drills, to open
//   the transaction or the detail report behind the figure.
// - The column header and the label column stay put while scrolling; on a phone
//   the grid scrolls sideways under the sticky labels.
// - Virtualized, so a 50,000-line general ledger renders instantly.

import { Text, UnstyledButton } from '@mantine/core';
import { IconChevronDown, IconChevronRight } from '@tabler/icons-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { formatDate, formatDateShort } from '~/lib/dates';
import { formatMoney } from '~/lib/money';
import { visibleRows } from './rows';
import type { Report, ReportCell, ReportColumn, ReportDrill } from './types';
import classes from './ReportViewer.module.css';

export interface ReportViewerProps {
  report: Report;
  collapsed: ReadonlySet<string>;
  onToggle(sectionId: string): void;
  onDrill(drill: ReportDrill): void;
  mobile?: boolean;
}

const ROW_H = 26;

function width(c: ReportColumn, i: number): string {
  if (i === 0) return c.kind === 'text' ? 'minmax(220px, 2.5fr)' : '110px';
  switch (c.kind) {
    case 'money':
    case 'quantity':
    case 'percent':
      return 'minmax(110px, 1fr)';
    case 'date':
      return '96px';
    default:
      return c.id === 'memo' ? 'minmax(160px, 2fr)' : 'minmax(100px, 1.2fr)';
  }
}

function cellText(cell: ReportCell, col: ReportColumn | undefined, mobile: boolean): string {
  if (cell === null || cell === undefined) return '';
  if (col?.kind === 'money' && typeof cell === 'number') return formatMoney(cell);
  if (col?.kind === 'date' && typeof cell === 'string') return mobile ? formatDateShort(cell) : formatDate(cell);
  return String(cell);
}

export function ReportViewer({ report, collapsed, onToggle, onDrill, mobile = false }: ReportViewerProps) {
  const scrollEl = useRef<HTMLDivElement>(null);
  const rows = useMemo(() => visibleRows(report.rows, collapsed), [report.rows, collapsed]);
  const [selected, setSelected] = useState(0);
  const template = report.columns.map(width).join(' ');
  const gridStyle = { gridTemplateColumns: template } as CSSProperties;
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollEl.current,
    estimateSize: () => ROW_H,
    overscan: 20,
  });

  const select = (i: number) => {
    const c = Math.max(0, Math.min(rows.length - 1, i));
    setSelected(c);
    virtualizer.scrollToIndex(c, { align: 'auto' });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const v = rows[selected];
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        select(selected + 1);
        break;
      case 'ArrowUp':
        e.preventDefault();
        select(selected - 1);
        break;
      case 'ArrowLeft':
        if (v?.collapsible && !v.collapsed) {
          e.preventDefault();
          onToggle(v.row.id);
        }
        break;
      case 'ArrowRight':
        if (v?.collapsible && v.collapsed) {
          e.preventDefault();
          onToggle(v.row.id);
        }
        break;
      case 'Enter':
        if (v?.row.drill) {
          e.preventDefault();
          onDrill(v.row.drill);
        } else if (v?.collapsible) {
          e.preventDefault();
          onToggle(v.row.id);
        }
        break;
      case 'Home':
      case 'End':
        e.preventDefault();
        select(e.key === 'Home' ? 0 : rows.length - 1);
        break;
    }
  };

  return (
    <div className={classes.root}>
      <div className={classes.titleBlock}>
        <Text size="sm" fw={600}>
          {report.company_name}
        </Text>
        <Text size="xl" fw={700}>
          {report.title}
        </Text>
        {report.subtitle && (
          <Text size="sm" c="dimmed">
            {report.subtitle}
          </Text>
        )}
        <Text size="xs" c="dimmed">
          {report.basis} Basis
        </Text>
      </div>
      <div
        ref={scrollEl}
        className={classes.scroll}
        tabIndex={0}
        role="grid"
        aria-label={report.title}
        aria-rowcount={rows.length}
        onKeyDown={onKeyDown}
        data-testid="report-grid"
      >
        <div className={classes.inner}>
          <div className={`${classes.row} ${classes.head}`} style={gridStyle} role="row">
            {report.columns.map((c, i) => (
              <div key={c.id} role="columnheader" className={i === 0 ? classes.sticky : c.kind === 'money' ? classes.num : undefined}>
                {c.label}
              </div>
            ))}
          </div>
          <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
            {virtualizer.getVirtualItems().map((vi) => {
              const v = rows[vi.index]!;
              const drill = v.row.drill;
              return (
                <div
                  key={v.row.id}
                  role="row"
                  className={classes.row}
                  data-kind={v.row.kind}
                  data-collapsed={v.collapsed || undefined}
                  data-selected={vi.index === selected || undefined}
                  data-drill={drill ? true : undefined}
                  style={{ ...gridStyle, position: 'absolute', top: 0, left: 0, right: 0, height: ROW_H, transform: `translateY(${vi.start}px)` }}
                  title={drill ? (mobile ? undefined : 'Double-click to QuickZoom') : undefined}
                  onClick={() => {
                    setSelected(vi.index);
                    if (mobile && drill) onDrill(drill);
                  }}
                  onDoubleClick={() => drill && onDrill(drill)}
                >
                  {v.cells.map((cell, i) => {
                    const col = report.columns[i];
                    if (i === 0) {
                      return (
                        <div key={i} role="gridcell" className={`${classes.sticky} ${classes.label}`} style={{ paddingLeft: 8 + v.row.depth * 16 }}>
                          {v.collapsible ? (
                            <UnstyledButton
                              className={classes.toggle}
                              tabIndex={-1}
                              aria-label={v.collapsed ? `Expand ${String(cell)}` : `Collapse ${String(cell)}`}
                              aria-expanded={!v.collapsed}
                              onClick={(e) => {
                                e.stopPropagation();
                                onToggle(v.row.id);
                              }}
                            >
                              {v.collapsed ? <IconChevronRight size={12} /> : <IconChevronDown size={12} />}
                            </UnstyledButton>
                          ) : null}
                          <span className={classes.ellipsis}>{cellText(cell, col, mobile)}</span>
                        </div>
                      );
                    }
                    const isMoney = col?.kind === 'money';
                    return (
                      <div
                        key={i}
                        role="gridcell"
                        className={`${isMoney ? classes.num : classes.ellipsis}`}
                        data-negative={isMoney && typeof cell === 'number' && cell < 0 ? true : undefined}
                      >
                        {cellText(cell, col, mobile)}
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
