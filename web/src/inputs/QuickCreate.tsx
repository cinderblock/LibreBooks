// Quick-create: what QuickFill's "<Add New>" (and QBD's "not in the list — Quick
// Add / Set Up") opens. One modal for the whole app; callers await the new id.

import { Alert, Button, Group, Modal, Select, SegmentedControl, Stack, TextInput } from '@mantine/core';
import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from 'react';
import { blankAccount, blankEntity, blankItem } from '~/api/blank';
import { describeError } from '~/api/errors';
import { useAccounts, useEntities, useSaveRecord } from '~/api/hooks';
import type { AccountType, EntityKind, ItemType } from '~/api/types';
import { ACCOUNT_TYPES, ACCOUNT_TYPE_LABEL } from '~/lib/accountTypes';
import { normalizeDecimalText } from '~/lib/decimal';
import { ITEM_TYPE_LABEL } from '~/lib/txnTypes';

type Request =
  | { kind: 'account'; name: string; types?: readonly AccountType[] }
  | { kind: 'entity'; name: string; kinds?: readonly EntityKind[] }
  | { kind: 'item'; name: string };

interface QuickCreateApi {
  createAccount(name: string, types?: readonly AccountType[]): Promise<string | null>;
  createEntity(name: string, kinds?: readonly EntityKind[]): Promise<string | null>;
  createItem(name: string): Promise<string | null>;
}

const QuickCreateContext = createContext<QuickCreateApi | null>(null);

export function useQuickCreate(): QuickCreateApi {
  const v = useContext(QuickCreateContext);
  if (!v) throw new Error('useQuickCreate outside <QuickCreateProvider>');
  return v;
}

const ENTITY_KIND_LABEL: Record<EntityKind, string> = { Customer: 'Customer', Vendor: 'Vendor', Employee: 'Employee', OtherName: 'Other' };

export function QuickCreateProvider({ children }: { children: ReactNode }) {
  const [req, setReq] = useState<Request | null>(null);
  const resolver = useRef<((id: string | null) => void) | null>(null);

  const ask = useCallback((r: Request) => {
    resolver.current?.(null);
    setReq(r);
    return new Promise<string | null>((resolve) => {
      resolver.current = resolve;
    });
  }, []);

  const finish = (id: string | null) => {
    resolver.current?.(id);
    resolver.current = null;
    setReq(null);
  };

  const api = useMemo<QuickCreateApi>(
    () => ({
      createAccount: (name, types) => ask({ kind: 'account', name, types }),
      createEntity: (name, kinds) => ask({ kind: 'entity', name, kinds }),
      createItem: (name) => ask({ kind: 'item', name }),
    }),
    [ask],
  );

  const title = req?.kind === 'account' ? 'New Account' : req?.kind === 'item' ? 'New Item' : 'New Name';
  return (
    <QuickCreateContext.Provider value={api}>
      {children}
      <Modal opened={req !== null} onClose={() => finish(null)} title={title} centered>
        {req?.kind === 'account' && <AccountForm req={req} onDone={finish} />}
        {req?.kind === 'entity' && <EntityForm req={req} onDone={finish} />}
        {req?.kind === 'item' && <ItemForm req={req} onDone={finish} />}
      </Modal>
    </QuickCreateContext.Provider>
  );
}

function Buttons({ busy, onCancel }: { busy: boolean; onCancel(): void }) {
  return (
    <Group justify="flex-end" mt="md">
      <Button variant="default" onClick={onCancel}>
        Cancel
      </Button>
      <Button type="submit" loading={busy}>
        Save
      </Button>
    </Group>
  );
}

function AccountForm({ req, onDone }: { req: Extract<Request, { kind: 'account' }>; onDone(id: string | null): void }) {
  const allowed = req.types?.length ? req.types : ACCOUNT_TYPES.filter((t) => t !== 'NonPosting');
  const [name, setName] = useState(req.name);
  const [type, setType] = useState<AccountType>(allowed[0]!);
  const [parentId, setParentId] = useState<string | null>(null);
  const [number, setNumber] = useState('');
  const { data: accounts = [] } = useAccounts();
  const save = useSaveRecord('accounts');
  const parents = accounts.filter((a) => a.account_type === type && a.is_active);
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const parent = accounts.find((a) => a.id === parentId) ?? null;
        const rec = { ...blankAccount(name.trim(), type, parent), number: number.trim() || null };
        save.mutate({ record: rec }, { onSuccess: (saved) => onDone(saved.id) });
      }}
    >
      <Stack gap="sm">
        <TextInput label="Account name" value={name} onChange={(e) => setName(e.currentTarget.value)} required data-autofocus />
        <Select
          label="Type"
          data={allowed.map((t) => ({ value: t, label: ACCOUNT_TYPE_LABEL[t] }))}
          value={type}
          onChange={(v) => {
            if (v) setType(v as AccountType);
            setParentId(null);
          }}
          allowDeselect={false}
        />
        <Select
          label="Subaccount of"
          placeholder="(top level)"
          data={parents.map((a) => ({ value: a.id, label: a.full_name }))}
          value={parentId}
          onChange={setParentId}
          clearable
          searchable
        />
        <TextInput label="Number" value={number} onChange={(e) => setNumber(e.currentTarget.value)} />
        {save.error && <Alert color="red">{describeError(save.error)}</Alert>}
      </Stack>
      <Buttons busy={save.isPending} onCancel={() => onDone(null)} />
    </form>
  );
}

function EntityForm({ req, onDone }: { req: Extract<Request, { kind: 'entity' }>; onDone(id: string | null): void }) {
  const kinds = req.kinds?.length ? req.kinds : (['Customer', 'Vendor', 'Employee', 'OtherName'] as const);
  const [name, setName] = useState(req.name);
  const [kind, setKind] = useState<EntityKind>(kinds[0]!);
  const [parentId, setParentId] = useState<string | null>(null);
  const { data: entities = [] } = useEntities();
  const save = useSaveRecord('entities');
  const customers = entities.filter((e) => e.kind === 'Customer' && e.is_active);
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const parent = kind === 'Customer' ? (entities.find((x) => x.id === parentId) ?? null) : null;
        save.mutate({ record: blankEntity(name.trim(), kind, parent) }, { onSuccess: (saved) => onDone(saved.id) });
      }}
    >
      <Stack gap="sm">
        <TextInput label="Name" value={name} onChange={(e) => setName(e.currentTarget.value)} required data-autofocus />
        {kinds.length > 1 && (
          <SegmentedControl data={kinds.map((k) => ({ value: k, label: ENTITY_KIND_LABEL[k] }))} value={kind} onChange={(v) => setKind(v as EntityKind)} />
        )}
        {kind === 'Customer' && (
          <Select
            label="Job of customer"
            placeholder="(a customer, not a job)"
            data={customers.map((c) => ({ value: c.id, label: c.full_name }))}
            value={parentId}
            onChange={setParentId}
            clearable
            searchable
          />
        )}
        {save.error && <Alert color="red">{describeError(save.error)}</Alert>}
      </Stack>
      <Buttons busy={save.isPending} onCancel={() => onDone(null)} />
    </form>
  );
}

const QUICK_ITEM_TYPES: ItemType[] = ['Service', 'NonInventory', 'OtherCharge'];

function ItemForm({ req, onDone }: { req: Extract<Request, { kind: 'item' }>; onDone(id: string | null): void }) {
  const [name, setName] = useState(req.name);
  const [type, setType] = useState<ItemType>('Service');
  const [description, setDescription] = useState('');
  const [price, setPrice] = useState('');
  const [account, setAccount] = useState<string | null>(null);
  const { data: accounts = [] } = useAccounts();
  const save = useSaveRecord('items');
  const postable = accounts.filter((a) => a.is_active && ['Income', 'Expense', 'CostOfGoodsSold', 'OtherIncome', 'OtherExpense'].includes(a.account_type));
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const acct = accounts.find((a) => a.id === account);
        const isIncome = acct?.account_type === 'Income' || acct?.account_type === 'OtherIncome';
        const rec = {
          ...blankItem(name.trim(), type),
          sales_description: description || null,
          purchase_description: description || null,
          sales_price: isIncome ? normalizeDecimalText(price) : null,
          purchase_cost: isIncome ? null : normalizeDecimalText(price),
          income_account_id: isIncome ? (account ?? null) : null,
          expense_account_id: isIncome ? null : (account ?? null),
        };
        save.mutate({ record: rec }, { onSuccess: (saved) => onDone(saved.id) });
      }}
    >
      <Stack gap="sm">
        <TextInput label="Item name" value={name} onChange={(e) => setName(e.currentTarget.value)} required data-autofocus />
        <Select
          label="Type"
          data={QUICK_ITEM_TYPES.map((t) => ({ value: t, label: ITEM_TYPE_LABEL[t] }))}
          value={type}
          onChange={(v) => v && setType(v as ItemType)}
          allowDeselect={false}
        />
        <TextInput label="Description" value={description} onChange={(e) => setDescription(e.currentTarget.value)} />
        <TextInput label="Rate (price or cost)" value={price} onChange={(e) => setPrice(e.currentTarget.value)} inputMode="decimal" />
        <Select
          label="Account"
          data={postable.map((a) => ({ value: a.id, label: a.full_name }))}
          value={account}
          onChange={setAccount}
          searchable
          required
        />
        {save.error && <Alert color="red">{describeError(save.error)}</Alert>}
      </Stack>
      <Buttons busy={save.isPending} onCancel={() => onDone(null)} />
    </form>
  );
}
