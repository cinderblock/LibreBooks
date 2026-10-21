// A company's information as one set of fields (`CompanyInfo`): the names,
// address and contact details, the fiscal year and tax form, and the
// preferences that shape the books. Shared by Company → Edit company
// information (`PUT /info`) and the picker's New company (`POST /companies`).

import { Group, NumberInput, Select, SimpleGrid, Stack, Switch, Text, TextInput } from '@mantine/core';
import type { CompanyInfo } from '~/api/types';
import { AddressFields } from '~/forms/records/AddressFields';

export const MONTHS = Array.from({ length: 12 }, (_, i) => ({ value: String(i + 1), label: new Date(2000, i, 1).toLocaleString(undefined, { month: 'long' }) }));
const TAX_FORMS = ['Form1040', 'Form1065', 'Form1120', 'Form1120S', 'Form990', 'Form990PF', 'Form990T', 'Other'];

export function CompanyInfoFields({ draft, onChange, fieldError, autoFocusName = true }: { draft: CompanyInfo; onChange(info: CompanyInfo): void; /** The server's 400 `invalid` message for a field, if it named one. */ fieldError?(field: string): string | undefined; autoFocusName?: boolean }) {
  const set = (patch: Partial<CompanyInfo>) => onChange({ ...draft, ...patch });
  const prefs = draft.preferences;
  const pref = (patch: Partial<CompanyInfo['preferences']>) => set({ preferences: { ...prefs, ...patch } });
  const err = (f: string) => fieldError?.(f);
  return (
    <Stack gap="sm">
      <SimpleGrid cols={{ base: 1, sm: 2 }}>
        <TextInput label="Company name" value={draft.name} onChange={(e) => set({ name: e.currentTarget.value })} required error={err('name')} data-autofocus={autoFocusName || undefined} aria-label="Company name" />
        <TextInput label="Legal name" value={draft.legal_name ?? ''} onChange={(e) => set({ legal_name: e.currentTarget.value || null })} aria-label="Legal name" />
      </SimpleGrid>
      <AddressFields label="Address" value={draft.address} onChange={(address) => set({ address })} />
      <SimpleGrid cols={{ base: 1, sm: 3 }}>
        <TextInput label="Phone" value={draft.phone ?? ''} onChange={(e) => set({ phone: e.currentTarget.value || null })} aria-label="Phone" />
        <TextInput label="Email" type="email" value={draft.email ?? ''} onChange={(e) => set({ email: e.currentTarget.value || null })} aria-label="Email" />
        <TextInput label="Website" value={draft.website ?? ''} onChange={(e) => set({ website: e.currentTarget.value || null })} aria-label="Website" />
      </SimpleGrid>
      <SimpleGrid cols={{ base: 1, sm: 3 }}>
        <TextInput label="EIN / tax id" value={draft.ein ?? ''} onChange={(e) => set({ ein: e.currentTarget.value || null })} aria-label="EIN" />
        <Select label="Fiscal year starts" data={MONTHS} value={String(draft.fiscal_year_start_month)} onChange={(v) => v && set({ fiscal_year_start_month: Number(v) })} allowDeselect={false} error={err('fiscal_year_start_month')} aria-label="Fiscal year starts" />
        <Select label="Income tax form" data={TAX_FORMS} value={draft.tax_form} onChange={(v) => set({ tax_form: v })} clearable aria-label="Income tax form" />
      </SimpleGrid>
      <Text fw={500} mt="xs">
        Preferences
      </Text>
      <SimpleGrid cols={{ base: 1, sm: 2 }}>
        <Switch label="Use account numbers" checked={prefs.use_account_numbers} onChange={(e) => pref({ use_account_numbers: e.currentTarget.checked })} />
        <Switch label="Class tracking" checked={prefs.use_class_tracking} onChange={(e) => pref({ use_class_tracking: e.currentTarget.checked })} />
        <Switch label="Sales tax" checked={prefs.sales_tax_enabled} onChange={(e) => pref({ sales_tax_enabled: e.currentTarget.checked })} />
        <Switch label="Inventory and purchase orders" checked={prefs.inventory_enabled} onChange={(e) => pref({ inventory_enabled: e.currentTarget.checked })} />
        <Switch label="Estimates" checked={prefs.estimates_enabled} onChange={(e) => pref({ estimates_enabled: e.currentTarget.checked })} />
        <Switch label="Deposit payments to Undeposited Funds by default" checked={prefs.use_undeposited_funds} onChange={(e) => pref({ use_undeposited_funds: e.currentTarget.checked })} />
        <Switch label="Auto-number journal entries" checked={prefs.auto_number_journal_entries} onChange={(e) => pref({ auto_number_journal_entries: e.currentTarget.checked })} />
        <Switch label="Reports default to cash basis" checked={prefs.reports_cash_basis} onChange={(e) => pref({ reports_cash_basis: e.currentTarget.checked })} />
      </SimpleGrid>
      <Group>
        <NumberInput label="Warn if a date is more than (days) in the past" value={prefs.warn_days_past ?? ''} onChange={(v) => pref({ warn_days_past: v === '' ? null : Number(v) })} min={0} w={260} />
        <NumberInput label="…or in the future" value={prefs.warn_days_future ?? ''} onChange={(v) => pref({ warn_days_future: v === '' ? null : Number(v) })} min={0} w={200} />
      </Group>
    </Stack>
  );
}
