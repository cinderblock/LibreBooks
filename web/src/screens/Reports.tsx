// Report Center (the catalog, grouped as QBD's Reports menu, plus memorized
// reports) and a report window: QBD's "Modify Report" options as a toolbar
// (dates, basis, columns, comparisons, percentages, sort, filters) over the
// ReportViewer, with QuickZoom drill-down, export (Excel, PDF on Letter or A4,
// CSV; Ctrl+E repeats the last pick), memorize and print (with Save as PDF).

import {
  ActionIcon,
  Alert,
  Badge,
  Button,
  Card,
  Checkbox,
  Group,
  Kbd,
  Loader,
  Menu,
  Modal,
  MultiSelect,
  Popover,
  SegmentedControl,
  Select,
  SimpleGrid,
  Skeleton,
  Stack,
  Text,
  TextInput,
  Title,
  Tooltip,
  UnstyledButton,
} from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { IconAdjustments, IconCheck, IconChevronDown, IconDeviceFloppy, IconDots, IconDownload, IconFileSpreadsheet, IconFileTypeCsv, IconFileTypePdf, IconFilter, IconPrinter, IconRefresh, IconSearch, IconTrash } from '@tabler/icons-react';
import { useEffect, useMemo, useState } from 'react';
import { useApi, useCompanyId } from '~/api/context';
import { type ExportPaper, saveDownload } from '~/api/client';
import { describeError } from '~/api/errors';
import { useAccountIndex, useAccounts, useClasses, useCompanyInfo, useDeleteSaved, useEntityIndex, useEntities, useItems, useReport, useReportCatalog, useSaved, useSaveSaved } from '~/api/hooks';
import type { Basis, CatalogEntry, ClearedFilter, ColumnLayout, DateMacro, Drill, ExportFormat, ReportFilters, ReportRequest, Saved, SortBy, TxnType } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { useComboHint, useShortcut } from '~/keyboard/KeyboardProvider';
import { ACCOUNT_TYPES, ACCOUNT_TYPE_LABEL } from '~/lib/accountTypes';
import { MACRO_LABELS, todayIso } from '~/lib/dates';
import { useExportFormat, useExportPaper } from '~/lib/prefs';
import { prepare, search } from '~/lib/quickfill';
import { TXN_LABEL, TXN_TYPES } from '~/lib/txnTypes';
import { useIsMobile } from '~/lib/useMobile';
import { ReportViewer } from '~/reports/ReportViewer';
import { EXPORT_FORMAT_INFO, EXPORT_FORMATS, EXPORT_PAPER_LABEL, EXPORT_PAPERS, exportFileName } from '~/reports/export';
import { COLUMN_LAYOUT_LABEL, EMPTY_FILTERS, filterCount, paramsToRequest, requestRange, requestToParams, SORT_LABEL, withDefaults } from '~/reports/request';
import { allSections, reportToCsv } from '~/reports/rows';
import { useOpenWindow, useWindow, useWindows } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { ShortcutButton } from '~/forms/common';
import classes from './screens.module.css';

const FORMAT_ICON: Record<ExportFormat, typeof IconDownload> = { Xlsx: IconFileSpreadsheet, Pdf: IconFileTypePdf, Csv: IconFileTypeCsv };

/** The export formats as menu items; the one the shortcut repeats shows its key. */
function ExportItems({ current, hint, disabled, onExport }: { current: ExportFormat; hint: string; disabled: boolean; onExport(format: ExportFormat): void }) {
  return EXPORT_FORMATS.map((f) => {
    const Icon = FORMAT_ICON[f];
    return (
      <Menu.Item key={f} leftSection={<Icon size={14} />} rightSection={hint && f === current ? <Kbd size="xs">{hint}</Kbd> : undefined} disabled={disabled} onClick={() => onExport(f)}>
        {EXPORT_FORMAT_INFO[f].label} (.{EXPORT_FORMAT_INFO[f].ext})
      </Menu.Item>
    );
  });
}

/** Letter / A4 for the PDF, ticked; choosing one leaves the menu open. */
function PaperItems({ paper, onChange }: { paper: ExportPaper; onChange(paper: ExportPaper): void }) {
  return EXPORT_PAPERS.map((p) => (
    <Menu.Item key={p} closeMenuOnClick={false} leftSection={p === paper ? <IconCheck size={14} /> : <span style={{ width: 14 }} />} onClick={() => onChange(p)}>
      {EXPORT_PAPER_LABEL[p]}
    </Menu.Item>
  ));
}

/** A saved report opens with its request, keyed by its own title. */
function memorizedPath(s: Saved): string {
  const req = s.doc as ReportRequest;
  return paths.report(req.report, requestToParams({ ...req, title: s.name }));
}

export function ReportCenter() {
  const open = useOpenWindow();
  const mobile = useIsMobile();
  const { data: catalog, isLoading, error } = useReportCatalog();
  const memorized = useSaved('memorized_report');
  const del = useDeleteSaved('memorized_report');
  const [query, setQuery] = useState('');
  const prepared = useMemo(() => prepare((catalog ?? []).map((r) => ({ fullName: r.name, keys: [r.description, r.category_label] }))), [catalog]);
  const groups = useMemo(() => {
    const shown = query.trim() ? search(prepared, query).map((i) => catalog![i]!) : (catalog ?? []);
    const m = new Map<string, CatalogEntry[]>();
    for (const r of shown) m.set(r.category_label, [...(m.get(r.category_label) ?? []), r]);
    return [...m];
  }, [catalog, prepared, query]);
  if (isLoading) return <Skeleton m="md" h={300} />;
  if (error) return <Alert m="md" color="red">{describeError(error)}</Alert>;
  return (
    <Stack p={mobile ? 'sm' : 'md'} gap="lg" className={classes.page} data-testid="report-center">
      <Group justify="space-between" wrap="wrap">
        {!mobile && <Title order={3}>Report Center</Title>}
        <TextInput
          placeholder="Find a report"
          leftSection={<IconSearch size={14} />}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          style={{ flex: mobile ? '1 1 100%' : '0 1 300px' }}
          aria-label="Find a report"
          autoFocus={!mobile}
        />
      </Group>
      {(memorized.data?.length ?? 0) > 0 && !query && (
        <div>
          <Text fw={600} mb="xs">
            Memorized Reports
          </Text>
          <SimpleGrid cols={{ base: 1, sm: 2, lg: 3 }} spacing="sm">
            {memorized.data!.map((s) => (
              <Card key={s.id} withBorder padding="sm" className={classes.clickable} onClick={() => open(memorizedPath(s))}>
                <Group justify="space-between" wrap="nowrap">
                  <div style={{ minWidth: 0 }}>
                    <Text fw={500} truncate>
                      {s.name}
                    </Text>
                    <Text size="xs" c="dimmed">
                      {catalog?.find((c) => c.id === (s.doc as ReportRequest).report)?.name ?? (s.doc as ReportRequest).report}
                    </Text>
                  </div>
                  <ActionIcon
                    variant="subtle"
                    color="gray"
                    aria-label={`Delete memorized report ${s.name}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (window.confirm(`Delete the memorized report "${s.name}"?`)) del.mutate(s.id);
                    }}
                  >
                    <IconTrash size={16} />
                  </ActionIcon>
                </Group>
              </Card>
            ))}
          </SimpleGrid>
        </div>
      )}
      {groups.map(([category, reports]) => (
        <div key={category}>
          <Text fw={600} mb="xs">
            {category}
          </Text>
          <SimpleGrid cols={{ base: 1, sm: 2, lg: 3 }} spacing="sm">
            {reports.map((r) => (
              <UnstyledButton key={r.id} onClick={() => open(paths.report(r.id))} aria-label={r.name}>
                <Card withBorder padding="sm" h="100%">
                  <Text fw={500}>{r.name}</Text>
                  <Text size="xs" c="dimmed" lineClamp={2}>
                    {r.description}
                  </Text>
                  {r.params.default_dates && (
                    <Text size="xs" c="dimmed" mt={4}>
                      {MACRO_LABELS.find(([m]) => m === r.params.default_dates)?.[1]}
                    </Text>
                  )}
                </Card>
              </UnstyledButton>
            ))}
          </SimpleGrid>
        </div>
      ))}
      {groups.length === 0 && <Text c="dimmed">No report matches.</Text>}
    </Stack>
  );
}

export function ReportWindow() {
  const win = useWindow();
  const windows = useWindows();
  const open = useOpenWindow();
  const api = useApi();
  const co = useCompanyId();
  const mobile = useIsMobile();
  const { data: info } = useCompanyInfo();
  const entities = useEntityIndex();
  const { data: catalog } = useReportCatalog();
  const entry = catalog?.find((c) => c.id === win.desc.params.report);
  const req = useMemo(() => withDefaults(paramsToRequest(win.desc.params), entry), [win.desc.params, entry]);
  const { data: report, error, isFetching, refetch } = useReport(catalog ? req : null);
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set());
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [memorizeOpen, setMemorizeOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const p = entry?.params;

  // The tab names the report and, when it's about one account or name, that subject.
  const accounts = useAccountIndex();
  useEffect(() => {
    const base = req.title ?? report?.title ?? entry?.name;
    if (!base) return;
    const f = req.filters;
    const subject = f.accounts.length === 1 ? accounts.get(f.accounts[0]!)?.full_name : f.names.length === 1 ? entities.get(f.names[0]!)?.full_name : null;
    win.setTitle(subject && !base.includes(subject) ? `${base}: ${subject}` : base);
  }, [report?.title, entry?.name, req.title, req.filters, accounts, entities]); // eslint-disable-line react-hooks/exhaustive-deps

  // Rows change when the request changes; a stale collapse set would hide the wrong rows.
  useEffect(() => setCollapsed(new Set()), [report?.rows]);

  // Changing options modifies this report in place (same window key).
  const change = (patch: Partial<ReportRequest>) => windows.open(paths.report(req.report, requestToParams({ ...req, ...patch })));
  const changeFilters = (patch: Partial<ReportFilters>) => change({ filters: { ...req.filters, ...patch } });
  const fsm = info?.fiscal_year_start_month ?? 1;
  const range = requestRange(req, todayIso(), fsm);
  const asOf = p?.dates === 'AsOf';

  const drill = (d: Drill) => {
    if (d.kind === 'Txn') open(paths.txn(d.txn_id));
    else if (d.kind === 'Report') open(paths.report(d.request.report, requestToParams(d.request)));
    else {
      const e = entities.get(d.entity_id);
      if (e?.kind === 'Vendor') open(paths.vendors(e.id));
      else if (e?.kind === 'Employee') open(paths.employees(e.id));
      else if (e?.kind === 'OtherName') open(paths.list('other-names'));
      else open(paths.customers(d.entity_id));
    }
  };

  // Export: the server renders the file (`POST /reports/export`); the pick becomes what Ctrl+E repeats.
  const [exportFormat, setExportFormat] = useExportFormat();
  const [paper, setPaper] = useExportPaper();
  const exportHint = useComboHint('report.export');
  const exportAs = async (format: ExportFormat) => {
    if (!report) return;
    setExportFormat(format);
    setExporting(true);
    try {
      saveDownload(await api.exportReport(co, req, format, format === 'Pdf' ? paper : undefined));
    } catch (e) {
      if (format === 'Csv') {
        // The server couldn't; the grid we have is just as good a CSV.
        notifications.show({ color: 'orange', message: `Export fell back to the on-screen report: ${describeError(e)}` });
        saveDownload({ blob: new Blob([reportToCsv(report)], { type: EXPORT_FORMAT_INFO.Csv.mime }), filename: exportFileName(req.title ?? report.title, 'Csv') });
      } else {
        notifications.show({ color: 'red', message: `Could not export the report: ${describeError(e)}` });
      }
    } finally {
      setExporting(false);
    }
  };

  const enabled = win.isActiveNow;
  useShortcut('record.print', () => window.print(), { enabled });
  useShortcut('report.memorize', () => setMemorizeOpen(true), { enabled });
  useShortcut('report.export', () => void exportAs(exportFormat), { enabled });

  const paperItems = <PaperItems paper={paper} onChange={setPaper} />;
  const exportItems = <ExportItems current={exportFormat} hint={mobile ? '' : exportHint} disabled={!report} onExport={(f) => void exportAs(f)} />;

  const allCollapsed = collapsed.size > 0;
  const comparisons = p?.comparisons && (req.previous_period || req.previous_year);
  const optionCount = [req.percent_of_income, req.percent_of_column, req.previous_period, req.previous_year, req.show_zero_rows].filter(Boolean).length;
  const nFilters = filterCount(req.filters);

  return (
    // aria-busy: the previous report stays on screen while a changed request loads (keepPreviousData); this says so.
    <div className={mobile ? classes.mobileFill : classes.fill} data-testid="report-window" aria-busy={isFetching}>
      <Group gap="xs" align="flex-end" wrap="wrap" px={mobile ? 'sm' : 0} pt={mobile ? 'xs' : 0}>
        {p && p.dates !== 'None' && (
          <>
            <Select
              label="Dates"
              size="xs"
              data={[...MACRO_LABELS.map(([value, label]) => ({ value, label })), { value: 'Custom', label: 'Custom' }]}
              value={req.date_macro ?? 'Custom'}
              onChange={(v) => (v && v !== 'Custom' ? change({ date_macro: v as DateMacro, from: null, to: null }) : change({ date_macro: null, from: range.from, to: range.to }))}
              allowDeselect={false}
              w={mobile ? '100%' : 190}
              aria-label="Dates"
            />
            {!asOf && <DateField size="xs" label="From" value={range.from} onChange={(from) => change({ date_macro: null, from, to: range.to })} aria-label="From" className={mobile ? classes.half : undefined} />}
            <DateField size="xs" label={asOf ? 'As of' : 'To'} value={range.to} onChange={(to) => change({ date_macro: null, from: asOf ? null : range.from, to })} aria-label={asOf ? 'As of' : 'To'} className={mobile ? classes.half : undefined} />
          </>
        )}
        {p?.basis && (
          <SegmentedControl
            size="xs"
            data={['Accrual', 'Cash']}
            value={req.basis ?? report?.basis ?? (info?.preferences.reports_cash_basis ? 'Cash' : 'Accrual')}
            onChange={(v) => change({ basis: v as Basis })}
            aria-label="Basis"
          />
        )}
        {p && p.columns.length > 1 && (
          <Select
            label="Columns"
            size="xs"
            data={p.columns.map((c) => ({ value: c, label: COLUMN_LAYOUT_LABEL[c] }))}
            value={req.columns ?? p.default_columns}
            onChange={(v) => v && change({ columns: v as ColumnLayout })}
            allowDeselect={false}
            w={150}
            aria-label="Columns"
          />
        )}
        {p && p.sorts.length > 1 && (
          <Select
            label="Sort by"
            size="xs"
            data={p.sorts.map((s) => ({ value: s, label: SORT_LABEL[s] }))}
            value={req.sort_by}
            onChange={(v) => v && change({ sort_by: v as SortBy })}
            allowDeselect={false}
            w={110}
            aria-label="Sort by"
          />
        )}
        {p && (p.comparisons || p.percent_of_income || p.percent_of_column || p.show_zero_rows) && (
          <Popover opened={optionsOpen} onChange={setOptionsOpen} position="bottom-start" withinPortal shadow="md" closeOnEscape={false}>
            <Popover.Target>
              <Button size="xs" variant="default" leftSection={<IconAdjustments size={14} />} rightSection={optionCount ? <Badge size="xs" circle>{optionCount}</Badge> : null} onClick={() => setOptionsOpen((o) => !o)}>
                Options
              </Button>
            </Popover.Target>
            {/* Esc closes the popover only; without this the window's Esc (close window) would see it too. */}
            <Popover.Dropdown
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault();
                  e.stopPropagation();
                  setOptionsOpen(false);
                }
              }}
            >
              <Stack gap={6}>
                {p.comparisons && (
                  <>
                    <Checkbox size="xs" label="Previous period" checked={req.previous_period} onChange={(e) => change({ previous_period: e.currentTarget.checked })} />
                    <Checkbox size="xs" label="Previous year" checked={req.previous_year} onChange={(e) => change({ previous_year: e.currentTarget.checked })} />
                    <Checkbox size="xs" label="$ change" checked={req.dollar_change} disabled={!comparisons} onChange={(e) => change({ dollar_change: e.currentTarget.checked })} />
                    <Checkbox size="xs" label="% change" checked={req.percent_change} disabled={!comparisons} onChange={(e) => change({ percent_change: e.currentTarget.checked })} />
                  </>
                )}
                {p.percent_of_income && <Checkbox size="xs" label="% of income" checked={req.percent_of_income} onChange={(e) => change({ percent_of_income: e.currentTarget.checked })} />}
                {p.percent_of_column && <Checkbox size="xs" label="% of column" checked={req.percent_of_column} onChange={(e) => change({ percent_of_column: e.currentTarget.checked })} />}
                {p.show_zero_rows && <Checkbox size="xs" label="Show rows with zero amounts" checked={req.show_zero_rows} onChange={(e) => change({ show_zero_rows: e.currentTarget.checked })} />}
              </Stack>
            </Popover.Dropdown>
          </Popover>
        )}
        {p && p.filters.length > 0 && (
          <Button size="xs" variant={nFilters ? 'light' : 'default'} leftSection={<IconFilter size={14} />} onClick={() => setFiltersOpen(true)} rightSection={nFilters ? <Badge size="xs" circle>{nFilters}</Badge> : null}>
            Filters
          </Button>
        )}
        {mobile ? (
          // A phone has no room for the row of buttons: one overflow menu holds them, the export formats among them.
          <Menu position="bottom-end" width={240}>
            <Menu.Target>
              <ActionIcon size="input-xs" variant="default" aria-label="More actions" loading={exporting}>
                <IconDots size={16} />
              </ActionIcon>
            </Menu.Target>
            <Menu.Dropdown>
              <Menu.Item onClick={() => setCollapsed(allCollapsed ? new Set() : allSections(report?.rows ?? []))} disabled={!report}>
                {allCollapsed ? 'Expand all' : 'Collapse all'}
              </Menu.Item>
              <Menu.Item leftSection={<IconRefresh size={14} />} onClick={() => void refetch()}>
                Refresh
              </Menu.Item>
              <Menu.Item leftSection={<IconDeviceFloppy size={14} />} onClick={() => setMemorizeOpen(true)} disabled={!report}>
                Memorize report
              </Menu.Item>
              <Menu.Divider />
              <Menu.Label>Export</Menu.Label>
              {exportItems}
              <Menu.Divider />
              <Menu.Label>PDF paper</Menu.Label>
              {paperItems}
            </Menu.Dropdown>
          </Menu>
        ) : (
          <Group gap="xs" ml="auto">
            <Button size="xs" variant="default" onClick={() => setCollapsed(allCollapsed ? new Set() : allSections(report?.rows ?? []))} disabled={!report}>
              {allCollapsed ? 'Expand' : 'Collapse'}
            </Button>
            <Tooltip label="Refresh">
              <ActionIcon size="input-xs" variant="default" aria-label="Refresh" onClick={() => void refetch()} loading={isFetching}>
                <IconRefresh size={14} />
              </ActionIcon>
            </Tooltip>
            <Tooltip label="Memorize this report (Ctrl+M)">
              <ActionIcon size="input-xs" variant="default" aria-label="Memorize report" onClick={() => setMemorizeOpen(true)} disabled={!report}>
                <IconDeviceFloppy size={14} />
              </ActionIcon>
            </Tooltip>
            <Menu position="bottom-end" width={240}>
              {/* No Tooltip between Menu.Target and the button: it would keep Menu's aria-expanded from the button, and Esc would then close the window. */}
              <Menu.Target>
                <Button size="xs" variant="default" leftSection={<IconDownload size={14} />} rightSection={<IconChevronDown size={12} />} disabled={!report} loading={exporting}>
                  Export
                </Button>
              </Menu.Target>
              <Menu.Dropdown>
                {exportItems}
                <Menu.Divider />
                <Menu.Label>PDF paper</Menu.Label>
                {paperItems}
              </Menu.Dropdown>
            </Menu>
            <Button.Group>
              <ShortcutButton size="xs" variant="default" command="record.print" leftSection={<IconPrinter size={14} />} onClick={() => window.print()}>
                Print
              </ShortcutButton>
              <Menu position="bottom-end" width={240}>
                <Menu.Target>
                  <ActionIcon size="input-xs" variant="default" aria-label="Print menu">
                    <IconChevronDown size={12} />
                  </ActionIcon>
                </Menu.Target>
                <Menu.Dropdown>
                  <Menu.Item leftSection={<IconFileTypePdf size={14} />} onClick={() => void exportAs('Pdf')} disabled={!report}>
                    Save as PDF ({EXPORT_PAPER_LABEL[paper]})
                  </Menu.Item>
                  <Menu.Divider />
                  <Menu.Label>Paper</Menu.Label>
                  {paperItems}
                </Menu.Dropdown>
              </Menu>
            </Button.Group>
          </Group>
        )}
      </Group>
      {error && <Alert color="red">{describeError(error)}</Alert>}
      {!report && !error && <Loader m="md" />}
      {report && (
        <ReportViewer
          report={report}
          collapsed={collapsed}
          mobile={mobile}
          onToggle={(i) =>
            setCollapsed((c) => {
              const n = new Set(c);
              if (n.has(i)) n.delete(i);
              else n.add(i);
              return n;
            })
          }
          onDrill={drill}
        />
      )}
      {p && <FiltersModal opened={filtersOpen} onClose={() => setFiltersOpen(false)} filters={req.filters} onChange={changeFilters} kinds={p.filters} />}
      <MemorizeReportModal opened={memorizeOpen} onClose={() => setMemorizeOpen(false)} request={req} defaultName={req.title ?? report?.title ?? entry?.name ?? ''} />
    </div>
  );
}

/** QBD's Filters tab: each list is "any of these"; empty means no filter. */
function FiltersModal({
  opened,
  onClose,
  filters,
  onChange,
  kinds,
}: {
  opened: boolean;
  onClose(): void;
  filters: ReportFilters;
  onChange(patch: Partial<ReportFilters>): void;
  kinds: CatalogEntry['params']['filters'];
}) {
  const { data: accounts = [] } = useAccounts();
  const { data: entities = [] } = useEntities();
  const { data: items = [] } = useItems();
  const { data: classList = [] } = useClasses();
  const has = (k: (typeof kinds)[number]) => kinds.includes(k);
  const opt = (rows: ReadonlyArray<{ id: string; full_name: string }>) => rows.map((r) => ({ value: r.id, label: r.full_name }));
  return (
    <Modal opened={opened} onClose={onClose} title="Filters" size="lg" centered>
      <Stack gap="sm">
        {has('Accounts') && <MultiSelect label="Accounts" data={opt(accounts)} value={filters.accounts} onChange={(accounts) => onChange({ accounts })} searchable clearable />}
        {has('AccountTypes') && (
          <MultiSelect
            label="Account types"
            data={ACCOUNT_TYPES.map((t) => ({ value: t, label: ACCOUNT_TYPE_LABEL[t] }))}
            value={filters.account_types}
            onChange={(v) => onChange({ account_types: v as ReportFilters['account_types'] })}
            searchable
            clearable
          />
        )}
        {has('Names') && <MultiSelect label="Names" data={opt(entities)} value={filters.names} onChange={(names) => onChange({ names })} searchable clearable />}
        {has('Classes') && <MultiSelect label="Classes" data={opt(classList)} value={filters.classes} onChange={(classes_) => onChange({ classes: classes_ })} searchable clearable />}
        {has('Items') && <MultiSelect label="Items" data={opt(items)} value={filters.items} onChange={(items_) => onChange({ items: items_ })} searchable clearable />}
        {has('TxnTypes') && (
          <MultiSelect
            label="Transaction types"
            data={TXN_TYPES.map((t) => ({ value: t, label: TXN_LABEL[t] }))}
            value={filters.txn_types}
            onChange={(v) => onChange({ txn_types: v as TxnType[] })}
            searchable
            clearable
          />
        )}
        <Group grow align="flex-end">
          {has('Cleared') && (
            <Select
              label="Cleared"
              data={[
                { value: 'Either', label: 'Either' },
                { value: 'Cleared', label: 'Cleared' },
                { value: 'NotCleared', label: 'Not cleared' },
              ]}
              value={filters.cleared ?? 'Either'}
              onChange={(v) => onChange({ cleared: v === 'Cleared' || v === 'NotCleared' ? (v as ClearedFilter) : null })}
              allowDeselect={false}
            />
          )}
          {has('Amount') && (
            <>
              <MoneyField label="Amount at least" value={filters.amount_min} onChange={(amount_min) => onChange({ amount_min })} />
              <MoneyField label="Amount at most" value={filters.amount_max} onChange={(amount_max) => onChange({ amount_max })} />
            </>
          )}
        </Group>
        {has('Memo') && <TextInput label="Memo contains" value={filters.memo ?? ''} onChange={(e) => onChange({ memo: e.currentTarget.value || null })} />}
        <Checkbox
          label="Exact: only the listed accounts, names, classes and items, not what's below them"
          checked={filters.exact}
          onChange={(e) => onChange({ exact: e.currentTarget.checked })}
        />
        <Group justify="space-between">
          <Button variant="subtle" color="gray" onClick={() => onChange({ ...EMPTY_FILTERS })}>
            Clear all
          </Button>
          <Button onClick={onClose}>Done</Button>
        </Group>
      </Stack>
    </Modal>
  );
}

/** Ctrl+M on a report: save the request under a name (QBD's Memorize Report). */
function MemorizeReportModal({ opened, onClose, request, defaultName }: { opened: boolean; onClose(): void; request: ReportRequest; defaultName: string }) {
  const windows = useWindows();
  const save = useSaveSaved('memorized_report');
  const memorized = useSaved('memorized_report');
  const existing = memorized.data?.find((s) => s.name === request.title);
  const [name, setName] = useState(defaultName);
  useEffect(() => {
    if (opened) setName(defaultName);
  }, [opened, defaultName]);
  const submit = (replace: boolean) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const doc: ReportRequest = { ...request, title: trimmed };
    save.mutate(
      { id: replace ? existing?.id : null, name: trimmed, doc },
      {
        onSuccess: () => {
          notifications.show({ message: `Memorized "${trimmed}".`, color: 'teal' });
          onClose();
          // The window now shows the memorized report, under its name.
          windows.open(paths.report(request.report, requestToParams(doc)));
        },
      },
    );
  };
  return (
    <Modal opened={opened} onClose={onClose} title="Memorize Report" centered>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit(false);
        }}
      >
        <Stack gap="sm">
          <TextInput label="Name" value={name} onChange={(e) => setName(e.currentTarget.value)} required data-autofocus />
          <Text size="xs" c="dimmed">
            The report's dates, columns, basis and filters are saved; it runs fresh each time you open it.
          </Text>
          {save.error && <Alert color="red">{describeError(save.error)}</Alert>}
          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            {existing && (
              <Button variant="light" onClick={() => submit(true)} loading={save.isPending}>
                Replace "{existing.name}"
              </Button>
            )}
            <Button type="submit" loading={save.isPending}>
              {existing ? 'Save as new' : 'Memorize'}
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}
