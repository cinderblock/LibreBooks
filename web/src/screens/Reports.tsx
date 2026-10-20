// Report Center (the catalog) and a report window: QBD's toolbar (Dates,
// From/To, basis, Collapse, Refresh, export, print) over the ReportViewer, with
// QuickZoom drill-down into transactions and detail reports.

import { Alert, Button, Card, Group, Loader, SegmentedControl, Select, SimpleGrid, Skeleton, Stack, Text, Title, UnstyledButton } from '@mantine/core';
import { IconDownload, IconPrinter, IconRefresh } from '@tabler/icons-react';
import { useEffect, useMemo, useState } from 'react';
import { describeError } from '~/api/errors';
import { useAccountIndex, useCompanyInfo, useEntityIndex, useReport, useReportCatalog } from '~/api/hooks';
import type { DateMacro } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { useShortcut } from '~/keyboard/KeyboardProvider';
import { MACRO_LABELS, macroRange, todayIso } from '~/lib/dates';
import { useIsMobile } from '~/lib/useMobile';
import { ReportViewer } from '~/reports/ReportViewer';
import { allSections, reportToCsv } from '~/reports/rows';
import type { ReportBasis, ReportDrill, ReportRequest } from '~/reports/types';
import { useOpenWindow, useWindow, useWindows } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import classes from './screens.module.css';

export function ReportCenter() {
  const open = useOpenWindow();
  const { data: catalog, isLoading, error } = useReportCatalog();
  const groups = useMemo(() => {
    const m = new Map<string, NonNullable<typeof catalog>>();
    for (const r of catalog ?? []) m.set(r.category, [...(m.get(r.category) ?? []), r]);
    return [...m];
  }, [catalog]);
  if (isLoading) return <Skeleton m="md" h={300} />;
  if (error) return <Alert m="md" color="red">{describeError(error)}</Alert>;
  return (
    <Stack p="md" gap="lg" className={classes.page}>
      <Title order={3}>Report Center</Title>
      {groups.map(([category, reports]) => (
        <div key={category}>
          <Text fw={600} mb="xs">
            {category}
          </Text>
          <SimpleGrid cols={{ base: 1, sm: 2, lg: 3 }} spacing="sm">
            {reports.map((r) => (
              <UnstyledButton key={r.id} onClick={() => open(paths.report(r.id, r.params.includes('account') ? {} : { macro: r.default_macro }))}>
              <Card withBorder padding="sm" h="100%">
                <Text fw={500}>{r.name}</Text>
                <Text size="xs" c="dimmed">
                  {MACRO_LABELS.find(([m]) => m === r.default_macro)?.[1]}
                  {r.params.includes('account') ? ' · for one account' : ''}
                </Text>
              </Card>
              </UnstyledButton>
            ))}
          </SimpleGrid>
        </div>
      ))}
    </Stack>
  );
}

/** The window's URL params as a request (and back). */
function requestFromParams(p: Record<string, string>): ReportRequest {
  return {
    report: p.report ?? '',
    macro: (p.macro as DateMacro | undefined) ?? null,
    range: p.from || p.to ? { from: p.from ?? null, to: p.to ?? null } : null,
    basis: (p.basis as ReportBasis | undefined) ?? null,
    account_id: p.account ?? null,
    entity_id: p.entity ?? null,
  };
}

function paramsFromRequest(r: ReportRequest): Record<string, string | null> {
  return {
    macro: r.macro ?? null,
    from: r.macro ? null : (r.range?.from ?? null),
    to: r.macro ? null : (r.range?.to ?? null),
    basis: r.basis ?? null,
    account: r.account_id ?? null,
    entity: r.entity_id ?? null,
  };
}

export function ReportWindow() {
  const win = useWindow();
  const windows = useWindows();
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const { data: info } = useCompanyInfo();
  const accounts = useAccountIndex();
  const entities = useEntityIndex();
  const { data: catalog } = useReportCatalog();
  const entry = catalog?.find((c) => c.id === win.desc.params.report);
  // No dates in the URL: the report's own default macro (shown as such, not as "Custom").
  const req = useMemo(() => {
    const r = requestFromParams(win.desc.params);
    return !r.macro && !r.range && entry ? { ...r, macro: entry.default_macro } : r;
  }, [win.desc.params, entry]);
  const { data: report, error, isFetching, refetch } = useReport(catalog ? req : null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!report) return;
    const subject = req.account_id ? accounts.get(req.account_id)?.full_name : req.entity_id ? entities.get(req.entity_id)?.full_name : null;
    win.setTitle(subject && !report.title.includes(subject) ? `${report.title}: ${subject}` : report.title);
  }, [report?.title, req.account_id, req.entity_id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Changing dates or basis modifies this report in place (same window).
  const change = (patch: Partial<ReportRequest>) => windows.open(paths.report(req.report, paramsFromRequest({ ...req, ...patch })));
  const fsm = info?.fiscal_year_start_month ?? 1;
  const range = req.macro ? macroRange(req.macro, todayIso(), fsm) : (req.range ?? report?.range ?? { from: null, to: null });

  const drill = (d: ReportDrill) => {
    if (d.kind === 'txn') open(paths.txn(d.txn_id));
    else if (d.kind === 'register') open(paths.register(d.account_id));
    else open(paths.report(d.request.report, paramsFromRequest(d.request)));
  };

  const exportCsv = () => {
    if (!report) return;
    const blob = new Blob([reportToCsv(report)], { type: 'text/csv;charset=utf-8' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${report.title.replace(/[^\w -]+/g, '').trim() || 'report'}.csv`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  };

  useShortcut('record.print', () => window.print(), { enabled: win.isActiveNow });

  const allCollapsed = report ? collapsed.size > 0 : false;
  return (
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="report-window">
      <Group gap="xs" align="flex-end" wrap="wrap" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0}>
        {(!entry || entry.params.includes('range')) && (
          <>
            <Select
              label="Dates"
              size="xs"
              data={[...MACRO_LABELS.map(([value, label]) => ({ value, label })), { value: 'Custom', label: 'Custom' }]}
              value={req.macro ?? 'Custom'}
              onChange={(v) => (v && v !== 'Custom' ? change({ macro: v as DateMacro, range: null }) : change({ macro: null, range }))}
              allowDeselect={false}
              w={190}
              aria-label="Dates"
            />
            <DateField size="xs" label="From" value={range.from} onChange={(from) => change({ macro: null, range: { from, to: range.to } })} aria-label="From" className={classes.num} />
            <DateField size="xs" label="To" value={range.to} onChange={(to) => change({ macro: null, range: { from: range.from, to } })} aria-label="To" />
          </>
        )}
        {(!entry || entry.params.includes('basis')) && (
          <SegmentedControl
            size="xs"
            data={['Accrual', 'Cash']}
            value={req.basis ?? report?.basis ?? (info?.preferences.reports_cash_basis ? 'Cash' : 'Accrual')}
            onChange={(v) => change({ basis: v as ReportBasis })}
            aria-label="Basis"
          />
        )}
        <Group gap="xs" ml={mobile ? 0 : 'auto'}>
          <Button size="xs" variant="default" onClick={() => setCollapsed(allCollapsed ? new Set() : allSections(report?.rows ?? []))} disabled={!report}>
            {allCollapsed ? 'Expand' : 'Collapse'}
          </Button>
          <Button size="xs" variant="default" leftSection={<IconRefresh size={14} />} onClick={() => void refetch()} loading={isFetching}>
            Refresh
          </Button>
          <Button size="xs" variant="default" leftSection={<IconDownload size={14} />} onClick={exportCsv} disabled={!report}>
            CSV
          </Button>
          {!mobile && (
            <Button size="xs" variant="default" leftSection={<IconPrinter size={14} />} onClick={() => window.print()}>
              Print
            </Button>
          )}
        </Group>
      </Group>
      {error && <Alert color="red">{describeError(error)}</Alert>}
      {!report && !error && <Loader m="md" />}
      {report && (
        <ReportViewer
          report={report}
          collapsed={collapsed}
          mobile={mobile}
          onToggle={(id) =>
            setCollapsed((c) => {
              const n = new Set(c);
              if (n.has(id)) n.delete(id);
              else n.add(id);
              return n;
            })
          }
          onDrill={drill}
        />
      )}
    </div>
  );
}
