// What each kind of window renders, and its title until the window sets a
// better one (a register names its account, a report its subject).

import type { ReactNode } from 'react';
import { WriteChecks } from '~/forms/WriteChecks';
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
import type { WindowDesc, WindowKind } from './model';

interface KindDef {
  render(desc: WindowDesc): ReactNode;
  title(desc: WindowDesc): string;
  /** Forms keep the window's state when it's hidden; lists and reports too. */
}

export const WINDOW_KINDS: Record<WindowKind, KindDef> = {
  home: { render: () => <Home />, title: () => 'Home' },
  accounts: { render: () => <ChartOfAccounts />, title: () => 'Chart of Accounts' },
  register: { render: () => <Register />, title: () => 'Register' },
  customers: { render: () => <Center which="customers" />, title: () => 'Customer Center' },
  vendors: { render: () => <Center which="vendors" />, title: () => 'Vendor Center' },
  employees: { render: () => <Center which="employees" />, title: () => 'Employee Center' },
  banking: { render: () => <Banking />, title: () => 'Banking' },
  check: { render: () => <WriteChecks />, title: () => 'Write Checks' },
  txn: { render: () => <TxnWindow />, title: () => 'Transaction' },
  reports: { render: () => <ReportCenter />, title: () => 'Report Center' },
  report: { render: () => <ReportWindow />, title: () => 'Report' },
  find: { render: () => <Find />, title: () => 'Find' },
  list: { render: () => <ListWindow />, title: (d) => LIST_TITLES[d.params.list ?? ''] ?? 'List' },
  company: { render: () => <Company />, title: () => 'Company' },
  form: { render: () => <FormPlaceholder />, title: (d) => FORM_LABELS[d.params.form ?? ''] ?? 'Form' },
};

export function windowTitle(desc: WindowDesc, titles: Record<string, string>): string {
  return titles[desc.key] ?? WINDOW_KINDS[desc.kind].title(desc);
}
