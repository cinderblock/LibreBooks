// The command palette (Ctrl+K): jump to any form, report, account, name or item,
// or to an open window, by typing part of its name. Ranked with the same
// QuickFill matcher the fields use, so `mat` finds "…:Materials" here too.

import { Spotlight, type SpotlightActionData, type SpotlightActionGroupData } from '@mantine/spotlight';
import { IconSearch } from '@tabler/icons-react';
import { useMemo, useState } from 'react';
import { useAccounts, useEntities, useItems, useReportCatalog } from '~/api/hooks';
import { ACCOUNT_TYPE_LABEL, hasRegister } from '~/lib/accountTypes';
import { prepare, search } from '~/lib/quickfill';
import { ITEM_TYPE_LABEL } from '~/lib/txnTypes';
import { useShortcutCatalog } from '~/keyboard/KeyboardProvider';
import { windowTitle } from '~/windows/catalog';
import { useWindows } from '~/windows/WindowsProvider';
import { paths } from '~/windows/paths';
import { allMenuItems } from './menus';
import type { RunCommand } from './GlobalCommands';

interface Entry {
  id: string;
  label: string;
  description: string;
  group: string;
  keys?: string[];
  run(): void;
}

const KIND_GROUP = { Customer: 'Customers & jobs', Vendor: 'Vendors', Employee: 'Employees', OtherName: 'Other names' } as const;
const CENTER = { Customer: paths.customers, Vendor: paths.vendors, Employee: paths.employees, OtherName: () => paths.list('other-names') } as const;

export function CommandPalette({ run }: { run: RunCommand }) {
  const windows = useWindows();
  const { format } = useShortcutCatalog();
  const { data: accounts = [] } = useAccounts();
  const { data: entities = [] } = useEntities();
  const { data: items = [] } = useItems();
  const { data: reports = [] } = useReportCatalog();
  const [query, setQuery] = useState('');

  const entries = useMemo<Entry[]>(() => {
    const out: Entry[] = [];
    for (const w of windows.state.windows) {
      out.push({ id: `win:${w.key}`, label: windowTitle(w, windows.state.titles), description: 'Open window', group: 'Open windows', run: () => windows.activate(w.key) });
    }
    for (const m of allMenuItems()) {
      const key = m.command ? format(m.command)[0] : undefined;
      out.push({
        id: `menu:${m.menu}:${m.label}`,
        label: m.label,
        description: `${m.menu}${key ? ` · ${key}` : ''}${m.todo ? ' · not built yet' : ''}`,
        group: 'Commands',
        keys: m.keywords,
        run: () => (m.path ? windows.open(m.path) : m.command ? run(m.command) : undefined),
      });
    }
    for (const r of reports) {
      // The QuickReport needs an account; it's reached from the accounts below.
      if (r.id === 'AccountQuickReport') continue;
      out.push({ id: `report:${r.id}`, label: r.name, description: `Report · ${r.category_label}`, group: 'Reports', keys: [r.description], run: () => windows.open(paths.report(r.id)) });
    }
    for (const a of accounts) {
      if (!a.is_active) continue;
      out.push({
        id: `acct:${a.id}`,
        label: a.full_name,
        description: `${ACCOUNT_TYPE_LABEL[a.account_type]} · ${hasRegister(a.account_type) ? 'register' : 'QuickReport'}`,
        group: 'Accounts',
        keys: a.number ? [a.number] : undefined,
        run: () => windows.open(hasRegister(a.account_type) ? paths.register(a.id) : paths.report('AccountQuickReport', { acct: a.id, dates: 'ThisFiscalYearToDate' })),
      });
    }
    for (const e of entities) {
      if (!e.is_active) continue;
      out.push({
        id: `ent:${e.id}`,
        label: e.full_name,
        description: e.kind === 'Customer' && e.parent_id ? 'Job' : e.kind === 'OtherName' ? 'Other name' : e.kind,
        group: KIND_GROUP[e.kind],
        keys: e.company_name ? [e.company_name] : undefined,
        run: () => windows.open(e.kind === 'OtherName' ? CENTER.OtherName() : CENTER[e.kind](e.id)),
      });
    }
    for (const i of items) {
      if (!i.is_active) continue;
      out.push({ id: `item:${i.id}`, label: i.full_name, description: `Item · ${ITEM_TYPE_LABEL[i.item_type]}`, group: 'Items', keys: i.sku ? [i.sku] : undefined, run: () => windows.open(paths.list('items')) });
    }
    return out;
  }, [windows, accounts, entities, items, reports, format, run]);

  const prepared = useMemo(() => prepare(entries.map((e) => ({ fullName: e.label, keys: e.keys }))), [entries]);

  // Rank here rather than with Spotlight's substring filter; cap what's rendered.
  // Groups appear in the order of their best match.
  const actions = useMemo<SpotlightActionGroupData[]>(() => {
    const order = query.trim() ? search(prepared, query) : entries.map((_, i) => i).filter((i) => entries[i]!.group === 'Open windows' || entries[i]!.group === 'Commands');
    const groups = new Map<string, SpotlightActionData[]>();
    for (const i of order.slice(0, 60)) {
      const e = entries[i]!;
      const list = groups.get(e.group) ?? [];
      list.push({ id: e.id, label: e.label, description: e.description, onClick: e.run });
      groups.set(e.group, list);
    }
    return [...groups].map(([group, list]) => ({ group, actions: list }));
  }, [entries, prepared, query]);

  return (
    <Spotlight
      actions={actions}
      query={query}
      onQueryChange={setQuery}
      filter={(_q, a) => a}
      shortcut={null}
      // No open animation: keys typed right after Ctrl+K must land in the search box.
      transitionProps={{ duration: 0 }}
      nothingFound="Nothing matches"
      highlightQuery
      scrollable
      maxHeight="60vh"
      searchProps={{ leftSection: <IconSearch size={18} />, placeholder: 'Go to a form, report, account, name or item…' }}
    />
  );
}
