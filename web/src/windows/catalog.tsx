// What each kind of window renders, and its title until the window sets a
// better one (a register names its account, a report its subject).

import { Button, Group, Stack, Text } from '@mantine/core';
import { useEffect, useState, type ReactNode } from 'react';
import { WriteChecks } from '~/forms/WriteChecks';
import { FORMS } from '~/forms/registry';
import { FormLoader } from '~/forms/useTxnForm';
import { AccountQuickFill } from '~/inputs/pickers';
import { Reconcile } from '~/screens/Reconcile';
import { Attachments } from '~/screens/Attachments';
import { BackupsWindow } from '~/screens/admin/Backups';
import { EnrollmentsWindow } from '~/screens/admin/Enrollments';
import { ImportWindow } from '~/screens/admin/Import';
import { ServerWindow } from '~/screens/admin/Server';
import { UsersWindow } from '~/screens/admin/Users';
import { BankFeeds } from '~/screens/feeds/BankFeeds';
import { MemorizedList } from '~/screens/Memorized';
import { MergeWindow } from '~/screens/Merge';
import { MyAccount } from '~/screens/MyAccount';
import { Banking } from '~/screens/Banking';
import { Center } from '~/screens/Centers';
import { ChartOfAccounts } from '~/screens/ChartOfAccounts';
import { Company } from '~/screens/Company';
import { Find } from '~/screens/Find';
import { Home } from '~/screens/Home';
import { LIST_TITLES, ListWindow } from '~/screens/Lists';
import { FormPlaceholder } from '~/screens/Placeholder';
import { Register } from '~/screens/Register';
import { ReportCenter, ReportWindow } from '~/screens/Reports';
import { TxnWindow } from '~/screens/TxnWindow';
import { FORM_LABELS } from '~/shell/menus';
import { useWindow, useWindows } from './WindowsProvider';
import type { WindowDesc, WindowKind } from './model';
import { paths } from './paths';

interface KindDef {
  render(desc: WindowDesc): ReactNode;
  title(desc: WindowDesc): string;
}

/** A `/forms/:slug/new` window: the registered form, or the placeholder for one not built yet. */
function FormWindow() {
  const win = useWindow();
  const slug = win.desc.params.form ?? '';
  if (slug === 'reconcile') return <ReconcilePicker />;
  const def = FORMS[slug];
  if (!def) return <FormPlaceholder />;
  return <FormLoader draftToken={win.desc.params.draft} render={(saved, initial) => def.render(saved, initial)} />;
}

function CheckWindow() {
  const win = useWindow();
  return <WriteChecks draftToken={win.desc.params.draft} />;
}

/** Banking → Reconcile: choose the account, then its reconcile window opens (and this closes). */
function ReconcilePicker() {
  const win = useWindow();
  const { open } = useWindows();
  const [id, setId] = useState<string | null>(null);
  useEffect(() => win.setTitle('Reconcile'), []); // eslint-disable-line react-hooks/exhaustive-deps
  const go = (accountId: string | null) => {
    if (!accountId) return;
    win.close({ force: true });
    open(paths.reconcile(accountId));
  };
  return (
    <Stack p="md" gap="sm" maw={480}>
      <Text>Select the bank or credit card account to reconcile.</Text>
      <AccountQuickFill label="Account" types={['Bank', 'CreditCard', 'OtherCurrentAsset', 'OtherCurrentLiability', 'LongTermLiability']} value={id} onChange={(v) => { setId(v); go(v); }} allowAddNew={false} autoFocus aria-label="Account to reconcile" />
      <Group justify="flex-end">
        <Button onClick={() => go(id)} disabled={!id}>
          Continue
        </Button>
      </Group>
    </Stack>
  );
}

export const WINDOW_KINDS: Record<WindowKind, KindDef> = {
  home: { render: () => <Home />, title: () => 'Home' },
  accounts: { render: () => <ChartOfAccounts />, title: () => 'Chart of Accounts' },
  register: { render: () => <Register />, title: () => 'Register' },
  customers: { render: () => <Center which="customers" />, title: () => 'Customer Center' },
  vendors: { render: () => <Center which="vendors" />, title: () => 'Vendor Center' },
  employees: { render: () => <Center which="employees" />, title: () => 'Employee Center' },
  banking: { render: () => <Banking />, title: () => 'Banking' },
  check: { render: () => <CheckWindow />, title: () => 'Write Checks' },
  txn: { render: () => <TxnWindow />, title: () => 'Transaction' },
  reports: { render: () => <ReportCenter />, title: () => 'Report Center' },
  report: { render: () => <ReportWindow />, title: () => 'Report' },
  find: { render: () => <Find />, title: () => 'Find' },
  list: { render: () => <ListWindow />, title: (d) => LIST_TITLES[d.params.list ?? ''] ?? 'List' },
  company: { render: () => <Company />, title: () => 'Company' },
  form: { render: () => <FormWindow />, title: (d) => FORMS[d.params.form ?? '']?.label ?? FORM_LABELS[d.params.form ?? ''] ?? 'Form' },
  reconcile: { render: () => <Reconcile />, title: () => 'Reconcile' },
  feeds: { render: () => <BankFeeds />, title: () => 'Bank Feeds' },
  memorized: { render: () => <MemorizedList />, title: () => 'Memorized Transaction List' },
  attachments: { render: () => <Attachments />, title: () => 'Attachments' },
  merge: { render: () => <MergeWindow />, title: () => 'Merge' },
  account: { render: () => <MyAccount />, title: () => 'My Account' },
  users: { render: () => <UsersWindow />, title: () => 'Users and Roles' },
  enrollments: { render: () => <EnrollmentsWindow />, title: () => 'Enrolment Links' },
  server: { render: () => <ServerWindow />, title: () => 'Server Settings' },
  backups: { render: () => <BackupsWindow />, title: () => 'Backups' },
  import: { render: () => <ImportWindow />, title: () => 'Import from QuickBooks' },
};

export function windowTitle(desc: WindowDesc, titles: Record<string, string>): string {
  return titles[desc.key] ?? WINDOW_KINDS[desc.kind].title(desc);
}
