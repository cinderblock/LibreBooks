// QuickFill bound to the company's lists: accounts, names, items, classes.
// Each builds its options once per list change (memoized), so a picker in every
// grid row costs one array, not one per row.

import { useMemo } from 'react';
import { useAccounts, useBalanceMap, useClasses, useEntities, useItems } from '~/api/hooks';
import type { Account, AccountType, Entity, EntityKind, Item } from '~/api/types';
import { ACCOUNT_TYPE_LABEL, isBalanceSheet, naturalBalance } from '~/lib/accountTypes';
import { formatMoney } from '~/lib/money';
import { ITEM_TYPE_LABEL } from '~/lib/txnTypes';
import { QuickFill, type QuickFillOption, type QuickFillProps } from './QuickFill';
import { useQuickCreate } from './QuickCreate';

type PickerProps = Omit<QuickFillProps, 'options' | 'onAddNew' | 'columnHeaders'> & { allowAddNew?: boolean };

export function accountOptions(accounts: readonly Account[], balances: Map<string, number>, types?: readonly AccountType[]): QuickFillOption[] {
  return accounts
    .filter((a) => !types || types.includes(a.account_type))
    .map((a) => ({
      id: a.id,
      fullName: a.full_name,
      name: a.name,
      depth: a.sublevel,
      inactive: !a.is_active,
      keys: a.number ? [a.number] : undefined,
      columns: [
        ACCOUNT_TYPE_LABEL[a.account_type],
        isBalanceSheet(a.account_type) ? formatMoney(naturalBalance(a.account_type, balances.get(a.id) ?? 0)) : '',
      ],
    }));
}

export function AccountQuickFill({ types, allowAddNew = true, ...props }: PickerProps & { types?: readonly AccountType[] }) {
  const { data: accounts = [] } = useAccounts();
  const balances = useBalanceMap();
  const qc = useQuickCreate();
  const typesKey = types?.join(',');
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const options = useMemo(() => accountOptions(accounts, balances, types), [accounts, balances, typesKey]);
  return (
    <QuickFill
      {...props}
      options={options}
      columnHeaders={['Type', 'Balance']}
      onAddNew={allowAddNew ? (name) => qc.createAccount(name, types) : undefined}
    />
  );
}

const KIND_LABEL: Record<EntityKind, string> = { Customer: 'Customer', Vendor: 'Vendor', Employee: 'Employee', OtherName: 'Other' };

export function entityOptions(entities: readonly Entity[], kinds?: readonly EntityKind[]): QuickFillOption[] {
  return entities
    .filter((e) => !kinds || kinds.includes(e.kind))
    .map((e) => ({
      id: e.id,
      fullName: e.full_name,
      name: e.name,
      depth: e.sublevel,
      inactive: !e.is_active,
      keys: e.company_name ? [e.company_name] : undefined,
      columns: [e.kind === 'Customer' && e.parent_id ? 'Job' : KIND_LABEL[e.kind]],
    }));
}

export function NameQuickFill({ kinds, allowAddNew = true, ...props }: PickerProps & { kinds?: readonly EntityKind[] }) {
  const { data: entities = [] } = useEntities();
  const qc = useQuickCreate();
  const kindsKey = kinds?.join(',');
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const options = useMemo(() => entityOptions(entities, kinds), [entities, kindsKey]);
  return (
    <QuickFill
      {...props}
      options={options}
      columnHeaders={['Type']}
      onAddNew={allowAddNew ? (name) => qc.createEntity(name, kinds) : undefined}
    />
  );
}

export function itemOptions(items: readonly Item[], side: 'buy' | 'sell'): QuickFillOption[] {
  return items.map((i) => ({
    id: i.id,
    fullName: i.full_name,
    name: i.name,
    depth: i.sublevel,
    inactive: !i.is_active,
    keys: i.sku ? [i.sku] : undefined,
    columns: [ITEM_TYPE_LABEL[i.item_type], (side === 'buy' ? i.purchase_cost : i.sales_price) ?? ''],
  }));
}

export function ItemQuickFill({ side = 'buy', allowAddNew = true, ...props }: PickerProps & { side?: 'buy' | 'sell' }) {
  const { data: items = [] } = useItems();
  const qc = useQuickCreate();
  const options = useMemo(() => itemOptions(items, side), [items, side]);
  return (
    <QuickFill
      {...props}
      options={options}
      columnHeaders={['Type', side === 'buy' ? 'Cost' : 'Price']}
      onAddNew={allowAddNew ? (name) => qc.createItem(name) : undefined}
    />
  );
}

export function ClassQuickFill(props: PickerProps) {
  const { data: classes = [] } = useClasses();
  const options = useMemo(
    () => classes.map((c) => ({ id: c.id, fullName: c.full_name, name: c.name, depth: c.sublevel, inactive: !c.is_active })),
    [classes],
  );
  return <QuickFill {...props} options={options} />;
}
