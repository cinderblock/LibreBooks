// ReportViewer: renders any lb-reports `Report` (columns with optional group
// headings, outlined rows, subtotals and totals).
//
// - Headers collapse/expand (click the triangle, or ←/→ on the selected row);
//   a collapsed header shows its subtotal on its own line, as QBD does.
// - QuickZoom: double-click (tap on a phone, or Enter) a row or a cell that
//   drills, to open the transaction, the detail report or the name behind it.
//   A cell's own drill (one month's figure) beats the row's.
// - The column header and the label column stay put while scrolling; on a phone
//   the grid scrolls sideways under the sticky labels.
// - Virtualized, so a 50,000-line general ledger renders instantly.

import { Text, UnstyledButton } from '@mantine/core';
import { IconChevronDown, IconChevronRight } from '@tabler/icons-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import type { Cell, Column, Drill, Report } from '~/api/types';
import { columnGroups, isNegative, valueText, visibleRows } from './rows';
import classes from './ReportViewer.module.css';

export interface ReportViewerProps {
  report: Report;
  /** Indexes of collapsed header rows. */
  collapsed: ReadonlySet<number>;
  onToggle(rowIndex: number): void;
  onDrill(drill: Drill): void;
  mobile?: boolean;
}

const ROW_H = 26;

const numeric = (c: Column) => c.kind === 'Money' || c.kind === 'Number' || c.kind === 'Percent';

function width(c: Column, i: number, mobile: boolean): string {
  // Labels ellipsize rather than push the figures off a phone's screen.
  if (i === 0) return c.kind === 'Text' ? (mobile ? 'minmax(150px, 2.5fr)' : 'minmax(220px, 2.5fr)') : '110px';
  switch (c.kind) {
    case 'Money':
      return mobile ? 'minmax(96px, 1fr)' : 'minmax(110px, 1fr)';
    case 'Number':
    case 'Percent':
      return mobile ? 'minmax(72px, 0.7fr)' : 'minmax(84px, 0.7fr)';
    case 'Date':
      return '96px';
    default:
      return c.id === 'memo' || c.id === 'description' ? 'minmax(160px, 2fr)' : 'minmax(100px, 1.2fr)';
  }
}

function alignClass(c: Column | undefined): string | undefined {
  if (!c) return undefined;
  if (c.align === 'Right' || (c.align !== 'Center' && numeric(c))) return classes.num;
  if (c.align === 'Center') return classes.center;
  return undefined;
}

export function ReportViewer({ report, collapsed, onToggle, onDrill, mobile = false }: ReportViewerProps) {
  const scrollEl = useRef<HTMLDivElement>(null);
  const rows = useMemo(() => visibleRows(report.rows, collapsed), [report.rows, collapsed]);
  const groups = useMemo(() => columnGroups(report.columns), [report.columns]);
  const hasGroups = groups.some((g) => g.label);
  const [selected, setSelected] = useState(0);
  const template = report.columns.map((c, i) => width(c, i, mobile)).join(' ');
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

  /** Where a row goes on Enter: its own drill, else its first cell's. */
  const rowDrill = (cells: Cell[], row: { drill?: Drill }): Drill | undefined => row.drill ?? cells.find((c) => c.drill)?.drill;

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
          onToggle(v.index);
        }
        break;
      case 'ArrowRight':
        if (v?.collapsible && v.collapsed) {
          e.preventDefault();
          onToggle(v.index);
        }
        break;
      case 'Enter': {
        const d = v ? rowDrill(v.cells, v.row) : undefined;
        if (d) {
          e.preventDefault();
          onDrill(d);
        } else if (v?.collapsible) {
          e.preventDefault();
          onToggle(v.index);
        }
        break;
      }
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
        <Text size="sm" c="dimmed">
          {report.subtitle}
        </Text>
        {report.basis && (
          <Text size="xs" c="dimmed">
            {report.basis} Basis
          </Text>
        )}
        {report.notes.length > 0 && (
          <ul className={classes.notes}>
            {report.notes.map((n) => (
              <li key={n}>{n}</li>
            ))}
          </ul>
        )}
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
          <div className={classes.head}>
            {hasGroups && (
              <div className={classes.row} style={gridStyle} role="row">
                {groups.map((g) => (
                  <div key={g.start} role="columnheader" className={`${classes.group} ${g.start === 0 ? classes.sticky : ''}`} style={{ gridColumn: `${g.start + 1} / span ${g.span}` }}>
                    {g.label}
                  </div>
                ))}
              </div>
            )}
            <div className={classes.row} style={gridStyle} role="row">
              {report.columns.map((c, i) => (
                <div key={c.id} role="columnheader" className={i === 0 ? classes.sticky : alignClass(c)}>
                  {c.label}
                </div>
              ))}
            </div>
          </div>
          <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
            {virtualizer.getVirtualItems().map((vi) => {
              const v = rows[vi.index]!;
              const drill = rowDrill(v.cells, v.row);
              return (
                <div
                  key={v.index}
                  role="row"
                  className={classes.row}
                  data-kind={v.row.kind}
                  data-collapsed={v.collapsed || undefined}
                  data-selected={vi.index === selected || undefined}
                  data-drill={drill ? true : undefined}
                  style={{ ...gridStyle, position: 'absolute', top: 0, left: 0, right: 0, height: ROW_H, transform: `translateY(${vi.start}px)` }}
                  title={drill && !mobile ? 'Double-click to QuickZoom' : undefined}
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
                              aria-label={v.collapsed ? `Expand ${valueText(cell.value)}` : `Collapse ${valueText(cell.value)}`}
                              aria-expanded={!v.collapsed}
                              onClick={(e) => {
                                e.stopPropagation();
                                onToggle(v.index);
                              }}
                            >
                              {v.collapsed ? <IconChevronRight size={12} /> : <IconChevronDown size={12} />}
                            </UnstyledButton>
                          ) : null}
                          <span className={classes.ellipsis}>{valueText(cell.value, { shortDate: mobile })}</span>
                        </div>
                      );
                    }
                    const own = cell.drill;
                    return (
                      <div
                        key={i}
                        role="gridcell"
                        className={`${alignClass(col) ?? classes.ellipsis} ${own ? classes.cellDrill : ''}`}
                        data-negative={isNegative(cell.value) || undefined}
                        onDoubleClick={
                          own
                            ? (e) => {
                                e.stopPropagation();
                                onDrill(own);
                              }
                            : undefined
                        }
                      >
                        {valueText(cell.value, { shortDate: mobile })}
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
