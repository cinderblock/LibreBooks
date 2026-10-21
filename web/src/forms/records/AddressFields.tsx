// A postal address as QBD's dialogs take it: free lines, then city / state /
// postal code / country. Shared by the name editors (bill-to, ship-to) and the
// company information forms.

import { SimpleGrid, Stack, Textarea, TextInput } from '@mantine/core';
import type { Address } from '~/api/types';

export function AddressFields({ label, value, onChange, prefix, withCountry = false, size }: { label: string; value: Address; onChange(a: Address): void; /** Distinguishes the two addresses' fields for assistive tech. */ prefix?: string; withCountry?: boolean; size?: 'xs' | 'sm' | 'md' }) {
  const set = (patch: Partial<Address>) => onChange({ ...value, ...patch });
  const name = (f: string) => (prefix ? `${prefix} ${f}` : f);
  const lines = (value.lines ?? []).join('\n');
  return (
    <Stack gap="xs">
      <Textarea label={label} value={lines} onChange={(e) => set({ lines: e.currentTarget.value.split('\n') })} autosize minRows={2} maxRows={5} size={size} aria-label={name('address lines')} />
      <SimpleGrid cols={{ base: 1, xs: withCountry ? 4 : 3 }} spacing="xs">
        <TextInput label="City" value={value.city ?? ''} onChange={(e) => set({ city: e.currentTarget.value || null })} size={size} aria-label={name('city')} />
        <TextInput label="State" value={value.state ?? ''} onChange={(e) => set({ state: e.currentTarget.value || null })} size={size} aria-label={name('state')} />
        <TextInput label="Postal code" value={value.postal_code ?? ''} onChange={(e) => set({ postal_code: e.currentTarget.value || null })} size={size} aria-label={name('postal code')} />
        {withCountry && <TextInput label="Country" value={value.country ?? ''} onChange={(e) => set({ country: e.currentTarget.value || null })} size={size} aria-label={name('country')} />}
      </SimpleGrid>
    </Stack>
  );
}

/** The address with blank lines dropped, as the API should get it. */
export function tidyAddress(a: Address): Address {
  const lines = (a.lines ?? []).map((l) => l.trim()).filter(Boolean);
  return { ...a, lines, city: a.city?.trim() || null, state: a.state?.trim() || null, postal_code: a.postal_code?.trim() || null, country: a.country?.trim() || null, note: a.note?.trim() || null };
}
