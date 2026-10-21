// New / Edit Customer, Job, Vendor, Employee and Other Name (the centres and
// the Other Names list: Ctrl+N, Ctrl+E). QBD's tabs: Address Info (the name,
// contact details, the addresses), Payment Settings (terms, credit limit, tax,
// the vendor's 1099 and account prefill), Additional Info (type, notes, custom
// fields) and, for a customer or job, Job Info.

import { Button, Checkbox, Group, Select, SimpleGrid, Stack, Tabs, Text, Textarea, TextInput, Title } from '@mantine/core';
import { useEffect, useMemo, useRef, useState } from 'react';
import { blankEntity } from '~/api/blank';
import { useEntities, useList, useSalesReps, useSalesTaxCodes, useTerms } from '~/api/hooks';
import type { Entity, EntityKind, JobStatus } from '~/api/types';
import { DateField } from '~/inputs/DateField';
import { MoneyField } from '~/inputs/MoneyField';
import { AccountQuickFill, entityOptions } from '~/inputs/pickers';
import { QuickFill } from '~/inputs/QuickFill';
import { ACCOUNT_TYPES } from '~/lib/accountTypes';
import { useIsMobile } from '~/lib/useMobile';
import { paths } from '~/windows/paths';
import { noErrors, type FormErrors } from '../model';
import { RecordFormActions, useRecordForm, type RecordFormSpec } from '../useRecordForm';
import classes from '../forms.module.css';
import { AddressFields, tidyAddress } from './AddressFields';
import { decimalError, decimalOrNull, fullNameOf, possibleParents, recordServerErrors, requireName, textOrNull } from './model';

export const ENTITY_KIND_LABEL: Record<EntityKind, string> = { Customer: 'Customer', Vendor: 'Vendor', Employee: 'Employee', OtherName: 'Other Name' };

const KINDS: readonly EntityKind[] = ['Customer', 'Vendor', 'Employee', 'OtherName'];
const isKind = (k: string | undefined): k is EntityKind => !!k && (KINDS as readonly string[]).includes(k);

const JOB_STATUS: Array<{ value: JobStatus; label: string }> = [
  { value: 'None', label: 'None' },
  { value: 'Pending', label: 'Pending' },
  { value: 'Awarded', label: 'Awarded' },
  { value: 'InProgress', label: 'In progress' },
  { value: 'Closed', label: 'Closed' },
  { value: 'NotAwarded', label: 'Not awarded' },
];

/** Accounts a vendor's bills and checks can prefill (anything that posts). */
const PREFILL_TYPES = ACCOUNT_TYPES.filter((t) => t !== 'NonPosting');

/** What this name is called in titles and toasts: a customer with a parent is a job. */
export function nameNoun(e: Entity): string {
  return e.kind === 'Customer' && e.parent_id ? 'job' : ENTITY_KIND_LABEL[e.kind].toLowerCase();
}

export function validateName(d: Entity): FormErrors {
  const e = noErrors();
  requireName(e, d.name, `the ${nameNoun(d)}'s name`);
  const pct = decimalError(d.price_level_percent);
  if (pct) e.header.price_level_percent = pct;
  if (d.job_start_date && d.job_end_date && d.job_end_date < d.job_start_date) e.header.job_end_date = 'The end date is before the start date.';
  if (d.hired_date && d.released_date && d.released_date < d.hired_date) e.header.released_date = 'The release date is before the hire date.';
  return e;
}

/** The record as the API takes it: trimmed text, nulls for blanks, tidy addresses. */
export function nameToRecord(d: Entity): Entity {
  const t = textOrNull;
  return {
    ...d,
    name: d.name.trim(),
    company_name: t(d.company_name),
    salutation: t(d.salutation),
    first_name: t(d.first_name),
    middle_name: t(d.middle_name),
    last_name: t(d.last_name),
    job_title: t(d.job_title),
    print_as: t(d.print_as),
    bill_address: tidyAddress(d.bill_address),
    ship_address: tidyAddress(d.ship_address),
    phone: t(d.phone),
    alt_phone: t(d.alt_phone),
    mobile: t(d.mobile),
    fax: t(d.fax),
    email: t(d.email),
    cc_email: t(d.cc_email),
    website: t(d.website),
    contact: t(d.contact),
    alt_contact: t(d.alt_contact),
    notes: t(d.notes),
    account_number: t(d.account_number),
    tax_id: t(d.tax_id),
    price_level_percent: decimalOrNull(d.price_level_percent),
    job_description: t(d.job_description),
    custom_fields: Object.fromEntries(Object.entries(d.custom_fields).filter(([k]) => k.trim())),
  };
}

const spec: RecordFormSpec<'entities', Entity> = {
  list: 'entities',
  noun: nameNoun,
  newDraft: (params) => blankEntity('', isKind(params.kind) ? params.kind : 'Customer'),
  fromRecord: (r) => r,
  toRecord: nameToRecord,
  validate: validateName,
  isEmpty: (d) => JSON.stringify(nameToRecord(d)) === JSON.stringify(nameToRecord({ ...blankEntity('', d.kind), id: d.id, parent_id: d.parent_id, sublevel: d.sublevel, full_name: d.full_name })),
  fingerprint: (d) => JSON.stringify(nameToRecord(d)),
  serverErrors: (_d, err) => recordServerErrors(err, { parent_id: 'parent', terms_id: 'terms', sales_tax_code_id: 'sales_tax_code' }),
  // Save & New keeps the kind (a vendor after a vendor), as QBD's dialogs do.
  nextDraft: (d) => blankEntity('', d.kind),
  newPath: (d) => paths.newRecord('entities', { kind: d.kind }),
  title: (d, saved) => {
    const what = saved ? nameNoun(saved) : nameNoun(d);
    const label = what === 'job' ? 'Job' : ENTITY_KIND_LABEL[d.kind];
    return saved ? `Edit ${label}: ${saved.full_name}` : `New ${label}`;
  },
  savedMessage: (r) => `${ENTITY_KIND_LABEL[r.kind] === 'Customer' && r.parent_id ? 'Job' : ENTITY_KIND_LABEL[r.kind]} "${r.full_name}" saved.`,
  attachmentKind: 'entity',
};

export function NameEditor({ saved, params }: { saved: Entity | null; params: Record<string, string> }) {
  const { data: entities = [] } = useEntities();
  const { data: terms = [] } = useTerms();
  const { data: taxCodes = [] } = useSalesTaxCodes();
  const { data: reps = [] } = useSalesReps();
  const { data: types = [] } = useList('types', { include_inactive: false });
  const firstField = useRef<HTMLInputElement>(null);
  const form = useRecordForm({ spec, saved, params, onSavedNew: () => firstField.current?.focus() });
  const { draft: d, update, errors, mobile } = form;
  const kind = d.kind;
  const customer = kind === 'Customer';
  const vendor = kind === 'Vendor';
  const employee = kind === 'Employee';
  const set = (patch: Partial<Entity>) => update((x) => ({ ...x, ...patch }));

  // "Add Job" from the Customer Center: the parent is preset once the names have loaded.
  const presetParent = params.parent;
  useEffect(() => {
    if (saved || !presetParent || d.parent_id || !entities.length) return;
    const p = entities.find((e) => e.id === presetParent && e.kind === 'Customer');
    if (p) set({ parent_id: p.id, sublevel: p.sublevel + 1, full_name: fullNameOf(d.name, p) });
  }, [presetParent, entities.length]); // eslint-disable-line react-hooks/exhaustive-deps

  const customers = useMemo(() => entityOptions(possibleParents(entities.filter((e) => e.kind === 'Customer'), saved)), [entities, saved]);
  const parent = d.parent_id ? entities.find((e) => e.id === d.parent_id) : undefined;
  const isJob = customer && !!d.parent_id;
  const label = isJob ? 'Job' : ENTITY_KIND_LABEL[kind];
  const typeOptions = types.filter((t) => t.list === (customer ? 'CustomerType' : vendor ? 'VendorType' : 'JobType')).map((t) => ({ value: t.id, label: t.full_name }));
  const jobTypeOptions = types.filter((t) => t.list === 'JobType').map((t) => ({ value: t.id, label: t.full_name }));
  const [tab, setTab] = useState<string | null>('address');

  const text = (key: keyof Entity, fieldLabel: string, extra: { placeholder?: string; w?: number } = {}) => (
    <TextInput label={fieldLabel} value={(d[key] as string | null) ?? ''} onChange={(e) => set({ [key]: e.currentTarget.value } as Partial<Entity>)} error={errors.header[key]} placeholder={extra.placeholder} w={extra.w} aria-label={fieldLabel} />
  );

  return (
    <div className={mobile ? classes.mobileForm : classes.form} style={mobile ? undefined : { maxWidth: 860 }} data-testid={`record-${kind.toLowerCase()}`} {...form.enterProps}>
      {form.banner}
      <Group justify="space-between" align="flex-start" wrap="wrap">
        <Title order={4}>{saved ? `Edit ${label}` : `New ${label}`}</Title>
        <Checkbox label={`${label} is inactive`} checked={!d.is_active} onChange={(e) => set({ is_active: !e.currentTarget.checked })} />
      </Group>
      <SimpleGrid cols={{ base: 1, sm: customer ? 2 : 1 }}>
        <TextInput
          label={`${label} name`}
          description={customer ? 'How the name reads in lists and on forms; the company name below prints on them.' : undefined}
          value={d.name}
          onChange={(e) => {
            const name = e.currentTarget.value;
            update((x) => ({ ...x, name, full_name: fullNameOf(name, parent) }));
          }}
          required
          error={errors.header.name}
          ref={firstField}
          autoFocus={!mobile}
          aria-label={`${label} name`}
        />
        {customer && (
          <QuickFill
            label="Job of"
            placeholder="(a customer, not a job)"
            options={customers}
            columnHeaders={['Type']}
            value={d.parent_id}
            onChange={(id) => {
              const p = id ? entities.find((e) => e.id === id) : undefined;
              set({ parent_id: p?.id ?? null, sublevel: p ? p.sublevel + 1 : 0, full_name: fullNameOf(d.name, p) });
            }}
            error={errors.header.parent}
            aria-label="Job of"
          />
        )}
      </SimpleGrid>
      <Tabs value={tab} onChange={setTab} keepMounted>
        <Tabs.List>
          <Tabs.Tab value="address">Address Info</Tabs.Tab>
          {(customer || vendor) && <Tabs.Tab value="payment">Payment Settings</Tabs.Tab>}
          <Tabs.Tab value="additional">Additional Info</Tabs.Tab>
          {customer && <Tabs.Tab value="job">Job Info</Tabs.Tab>}
          {employee && <Tabs.Tab value="employment">Employment</Tabs.Tab>}
        </Tabs.List>
        <Tabs.Panel value="address" pt="sm">
          <Stack gap="sm">
            <SimpleGrid cols={{ base: 1, sm: 2 }}>
              {text('company_name', 'Company name')}
              {text('job_title', 'Job title')}
            </SimpleGrid>
            <Group gap="xs" align="flex-end" wrap={mobile ? 'wrap' : 'nowrap'}>
              {text('salutation', 'Mr./Ms.', { w: mobile ? undefined : 90 })}
              <TextInput label="First name" value={d.first_name ?? ''} onChange={(e) => set({ first_name: e.currentTarget.value })} style={{ flex: 1 }} aria-label="First name" />
              {text('middle_name', 'M.I.', { w: mobile ? undefined : 70 })}
              <TextInput label="Last name" value={d.last_name ?? ''} onChange={(e) => set({ last_name: e.currentTarget.value })} style={{ flex: 1 }} aria-label="Last name" />
            </Group>
            <SimpleGrid cols={{ base: 1, sm: 3 }}>
              {text('phone', 'Main phone')}
              {text('alt_phone', 'Work phone')}
              {text('mobile', 'Mobile')}
              {text('fax', 'Fax')}
              {text('email', 'Main email')}
              {text('cc_email', 'CC email')}
              {text('website', 'Website')}
              {text('contact', 'Contact')}
              {text('alt_contact', 'Alternate contact')}
            </SimpleGrid>
            <SimpleGrid cols={{ base: 1, md: customer ? 2 : 1 }} spacing="md">
              <AddressFields label={customer ? 'Invoice / bill to' : vendor ? 'Billed from' : 'Address'} prefix="Bill-to" value={d.bill_address} onChange={(bill_address) => set({ bill_address })} />
              {customer && (
                <Stack gap={4}>
                  <AddressFields label="Ship to" prefix="Ship-to" value={d.ship_address} onChange={(ship_address) => set({ ship_address })} />
                  <Button size="compact-xs" variant="subtle" style={{ alignSelf: 'flex-start' }} onClick={() => set({ ship_address: structuredClone(d.bill_address) })}>
                    Copy from bill to
                  </Button>
                </Stack>
              )}
            </SimpleGrid>
          </Stack>
        </Tabs.Panel>
        {(customer || vendor) && (
          <Tabs.Panel value="payment" pt="sm">
            <Stack gap="sm">
              <SimpleGrid cols={{ base: 1, sm: 3 }}>
                {text('account_number', customer ? 'Account no. (theirs with us)' : 'Account no. (ours with them)')}
                <Select label="Payment terms" data={terms.map((t) => ({ value: t.id, label: t.name }))} value={d.terms_id} onChange={(terms_id) => set({ terms_id })} clearable searchable error={errors.header.terms} aria-label="Payment terms" />
                <MoneyField label="Credit limit" value={d.credit_limit} onChange={(credit_limit) => set({ credit_limit })} allowNegative={false} aria-label="Credit limit" />
              </SimpleGrid>
              {customer && (
                <SimpleGrid cols={{ base: 1, sm: 3 }}>
                  <TextInput label="Price level (%)" description="A markup (+) or discount (−) on item prices" value={d.price_level_percent ?? ''} onChange={(e) => set({ price_level_percent: e.currentTarget.value })} inputMode="decimal" error={errors.header.price_level_percent} aria-label="Price level percent" />
                  <Select label="Sales rep" data={reps.map((r) => ({ value: r.id, label: r.initials }))} value={d.sales_rep_id} onChange={(sales_rep_id) => set({ sales_rep_id })} clearable aria-label="Sales rep" />
                  <Select label="Sales tax code" data={taxCodes.filter((c) => c.is_active || c.id === d.sales_tax_code_id).map((c) => ({ value: c.id, label: `${c.name}${c.description ? ` · ${c.description}` : ''}` }))} value={d.sales_tax_code_id} onChange={(sales_tax_code_id) => set({ sales_tax_code_id })} clearable error={errors.header.sales_tax_code} aria-label="Sales tax code" />
                </SimpleGrid>
              )}
              {customer && text('tax_id', 'Resale number')}
              {vendor && (
                <>
                  <SimpleGrid cols={{ base: 1, sm: 2 }}>
                    {text('print_as', 'Print on check as', { placeholder: d.company_name ?? d.name })}
                    {text('tax_id', 'Tax ID (EIN or SSN)')}
                  </SimpleGrid>
                  <Checkbox label="Vendor is eligible for a 1099" checked={d.is_1099} onChange={(e) => set({ is_1099: e.currentTarget.checked })} />
                  <AccountQuickFill label="Prefill bills and checks with this account" types={PREFILL_TYPES} value={d.default_expense_account_id} onChange={(default_expense_account_id) => set({ default_expense_account_id })} allowAddNew aria-label="Default expense account" />
                </>
              )}
            </Stack>
          </Tabs.Panel>
        )}
        <Tabs.Panel value="additional" pt="sm">
          <Stack gap="sm">
            {(customer || vendor) && <Select label={`${label} type`} data={typeOptions} value={d.type_id} onChange={(type_id) => set({ type_id })} clearable searchable aria-label={`${label} type`} />}
            <Textarea label="Notes" value={d.notes ?? ''} onChange={(e) => set({ notes: e.currentTarget.value })} autosize minRows={3} aria-label="Notes" />
            <CustomFields value={d.custom_fields} onChange={(custom_fields) => set({ custom_fields })} />
          </Stack>
        </Tabs.Panel>
        {customer && (
          <Tabs.Panel value="job" pt="sm">
            <Stack gap="sm">
              <SimpleGrid cols={{ base: 1, sm: 2 }}>
                <Select label="Job status" data={JOB_STATUS} value={d.job_status ?? 'None'} onChange={(v) => set({ job_status: (v as JobStatus | null) ?? 'None' })} allowDeselect={false} aria-label="Job status" />
                <Select label="Job type" data={jobTypeOptions} value={d.job_type_id} onChange={(job_type_id) => set({ job_type_id })} clearable searchable aria-label="Job type" />
              </SimpleGrid>
              <SimpleGrid cols={{ base: 1, sm: 3 }}>
                <DateField label="Start date" value={d.job_start_date} onChange={(job_start_date) => set({ job_start_date })} aria-label="Job start date" />
                <DateField label="Projected end" value={d.job_projected_end} onChange={(job_projected_end) => set({ job_projected_end })} aria-label="Projected end date" />
                <DateField label="End date" value={d.job_end_date} onChange={(job_end_date) => set({ job_end_date })} error={errors.header.job_end_date} aria-label="Job end date" />
              </SimpleGrid>
              <Textarea label="Job description" value={d.job_description ?? ''} onChange={(e) => set({ job_description: e.currentTarget.value })} autosize minRows={2} aria-label="Job description" />
            </Stack>
          </Tabs.Panel>
        )}
        {employee && (
          <Tabs.Panel value="employment" pt="sm">
            <SimpleGrid cols={{ base: 1, sm: 3 }}>
              <DateField label="Hire date" value={d.hired_date} onChange={(hired_date) => set({ hired_date })} aria-label="Hire date" />
              <DateField label="Release date" value={d.released_date} onChange={(released_date) => set({ released_date })} error={errors.header.released_date} aria-label="Release date" />
            </SimpleGrid>
          </Tabs.Panel>
        )}
      </Tabs>
      {form.formErrors}
      <RecordFormActions form={form} />
      {form.chrome}
    </div>
  );
}

/** QBD's "Define Fields": free-form labelled values on a name. */
function CustomFields({ value, onChange }: { value: Record<string, string>; onChange(v: Record<string, string>): void }) {
  const [newName, setNewName] = useState('');
  const mobile = useIsMobile();
  const keys = Object.keys(value);
  const add = () => {
    const k = newName.trim();
    if (!k || k in value) return;
    onChange({ ...value, [k]: '' });
    setNewName('');
  };
  return (
    <Stack gap="xs">
      <Text size="sm" fw={500}>
        Custom fields
      </Text>
      {keys.length > 0 && (
        <SimpleGrid cols={{ base: 1, sm: 2 }}>
          {keys.map((k) => (
            <TextInput
              key={k}
              label={k}
              value={value[k] ?? ''}
              onChange={(e) => onChange({ ...value, [k]: e.currentTarget.value })}
              rightSection={
                <Button size="compact-xs" variant="subtle" color="gray" onClick={() => onChange(Object.fromEntries(Object.entries(value).filter(([x]) => x !== k)))} aria-label={`Remove ${k}`}>
                  ×
                </Button>
              }
              aria-label={`Custom field ${k}`}
            />
          ))}
        </SimpleGrid>
      )}
      <Group gap="xs" align="flex-end" wrap={mobile ? 'wrap' : 'nowrap'}>
        <TextInput
          label="New field"
          placeholder="Birthday, Spouse, Contract no.…"
          value={newName}
          onChange={(e) => setNewName(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          style={{ flex: 1 }}
          aria-label="New custom field name"
        />
        <Button variant="default" onClick={add} disabled={!newName.trim()}>
          Add field
        </Button>
      </Group>
    </Stack>
  );
}
