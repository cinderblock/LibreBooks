// A list record's editor window (`/lists/:list/new`, `/lists/:list/:id`): the
// editor for the list, loading the saved record first. Lists without an
// editor yet (sales reps, customer/vendor/job types) say so.

import { Alert } from '@mantine/core';
import { AccountEditor } from '~/forms/records/AccountEditor';
import { ItemEditor } from '~/forms/records/ItemEditor';
import { ENTITY_KIND_LABEL, NameEditor } from '~/forms/records/NameEditor';
import { ClassEditor, PaymentMethodEditor, SalesTaxCodeEditor, TermsEditor } from '~/forms/records/SmallListEditors';
import { RecordLoader } from '~/forms/useRecordForm';
import type { EntityKind } from '~/api/types';
import { useWindow } from '~/windows/WindowsProvider';

const NOUN: Record<string, string> = {
  accounts: 'Account',
  entities: 'Name',
  items: 'Item',
  classes: 'Class',
  terms: 'Terms',
  'payment-methods': 'Payment Method',
  'sales-tax-codes': 'Sales Tax Code',
};

/** The window's title until the editor sets a better one (the record's name). */
export function recordWindowTitle(params: Record<string, string>): string {
  const list = params.list ?? '';
  let noun = NOUN[list] ?? 'Record';
  if (list === 'entities' && params.kind && params.kind in ENTITY_KIND_LABEL) noun = ENTITY_KIND_LABEL[params.kind as EntityKind];
  return `${params.recordId ? 'Edit' : 'New'} ${noun}`;
}

export function RecordWindow() {
  const win = useWindow();
  const params = win.desc.params;
  const list = params.list ?? '';
  const id = params.recordId ?? null;
  switch (list) {
    case 'accounts':
      return <RecordLoader list="accounts" recordId={id} render={(saved) => <AccountEditor saved={saved} params={params} />} />;
    case 'entities':
      return <RecordLoader list="entities" recordId={id} render={(saved) => <NameEditor saved={saved} params={params} />} />;
    case 'items':
      return <RecordLoader list="items" recordId={id} render={(saved) => <ItemEditor saved={saved} params={params} />} />;
    case 'classes':
      return <RecordLoader list="classes" recordId={id} render={(saved) => <ClassEditor saved={saved} params={params} />} />;
    case 'terms':
      return <RecordLoader list="terms" recordId={id} render={(saved) => <TermsEditor saved={saved} params={params} />} />;
    case 'payment-methods':
      return <RecordLoader list="payment-methods" recordId={id} render={(saved) => <PaymentMethodEditor saved={saved} params={params} />} />;
    case 'sales-tax-codes':
      return <RecordLoader list="sales-tax-codes" recordId={id} render={(saved) => <SalesTaxCodeEditor saved={saved} params={params} />} />;
    default:
      return <Alert m="md">There's no editor for the "{list}" list yet.</Alert>;
  }
}
