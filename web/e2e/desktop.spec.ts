import { expect, test, type Page } from '@playwright/test';

// Fixture mode with no simulated latency; the browser keymap (a plain browser).
async function start(page: Page, path = '/') {
  await page.goto(`${path}?fixtures=1&latency=0&keymap=browser`);
  await expect(page.getByRole('menubar', { name: 'Main menu' })).toBeVisible();
}

test('home shows balances, the menu bar and the window tabs', async ({ page }) => {
  await start(page);
  await expect(page.getByText('Bank and credit card accounts')).toBeVisible();
  await expect(page.getByRole('tab', { name: /Home/ })).toBeVisible();
  for (const m of ['Company', 'Customers', 'Vendors', 'Banking', 'Reports', 'Lists', 'Window', 'Help']) {
    await expect(page.getByRole('menubar').getByText(m, { exact: true })).toBeVisible();
  }
  await page.screenshot({ path: test.info().outputPath('home-desktop.png') });
});

test('write a check entirely from the keyboard, then find it in the register', async ({ page }) => {
  await start(page);
  // Browser keymap: Write Checks is Alt+Shift+W (Ctrl+W would close the tab).
  await page.keyboard.press('Alt+Shift+W');
  const form = page.getByTestId('write-checks');
  await expect(form).toBeVisible();
  const payee = form.getByRole('combobox', { name: 'Payee' });
  await expect(payee).toBeFocused();
  await payee.pressSequentially('Office D');
  await expect(payee).toHaveValue('Office Depot'); // inline QuickFill completion
  await page.keyboard.press('Tab');
  // The vendor's default expense account lands on the first line.
  await expect(form.getByRole('combobox', { name: 'Account, line 1' })).toHaveValue('Office Supplies');
  // The calculator: 12*3+4 = → 40.00, and the amount goes onto the line.
  const amount = form.getByRole('textbox', { name: 'Check amount' });
  await amount.click();
  await amount.pressSequentially('12*3+4');
  await page.keyboard.press('=');
  await expect(amount).toHaveValue('40.00');
  await expect(form.getByRole('textbox', { name: 'Amount, line 1' })).toHaveValue('40.00');
  await expect(form.getByLabel('Amount in words')).toContainText('Forty and 00/100');
  await page.keyboard.press('Alt+A'); // Save & Close
  await expect(page.getByText(/Check \d+ for \$40\.00 saved/)).toBeVisible();
  await expect(form).toBeHidden();

  // Ctrl+K → "Checking" → its register shows the new check.
  await page.keyboard.press('Control+K');
  await expect(page.getByPlaceholder(/Go to a form/)).toBeFocused();
  await page.keyboard.type('Checking');
  await expect(page.getByRole('button', { name: /^Checking/ }).first()).toBeVisible();
  await page.keyboard.press('Enter');
  const grid = page.getByTestId('register-grid');
  await expect(grid).toBeVisible();
  await expect(grid.getByText('Office Depot').last()).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('register-desktop.png') });
});

test('enter a bill: the vendor brings its terms, the due date follows, Save & New clears', async ({ page }) => {
  await start(page);
  await page.getByRole('menubar').getByText('Vendors', { exact: true }).click();
  await page.getByRole('menuitem', { name: /Enter Bills/ }).click();
  const form = page.getByTestId('form-bill');
  await expect(form).toBeVisible();
  const vendor = form.getByRole('combobox', { name: 'Vendor' });
  await expect(vendor).toBeFocused();
  await vendor.pressSequentially("Bob's");
  await expect(vendor).toHaveValue("Bob's Finishing");
  await page.keyboard.press('Tab');
  // Bob's Finishing is on Net 15: the terms and the due date fill in.
  await expect(form.getByRole('combobox', { name: 'Terms' })).toHaveValue('Net 15');
  const date = await form.getByRole('textbox', { name: 'Date' }).inputValue();
  const due = await form.getByRole('textbox', { name: 'Bill due' }).inputValue();
  expect(due).not.toBe(date);
  // The vendor's last bill is recalled (its subcontractor line), as QBD does.
  await expect(form.getByRole('combobox', { name: 'Account, line 1' })).toHaveValue('Cost of Goods Sold:Subcontractors');
  await expect(form.getByRole('textbox', { name: 'Amount, line 1' })).not.toHaveValue('');
  // Change the line's amount; the amount due follows the lines.
  const line = form.getByRole('textbox', { name: 'Amount, line 1' });
  await line.click();
  await line.fill('1250');
  await page.keyboard.press('Tab');
  await expect(form.getByRole('textbox', { name: 'Amount Due' })).toHaveValue('1,250.00');
  await form.getByRole('button', { name: /Save & New/ }).click();
  await expect(page.getByText(/Bill .*for \$1,250\.00 saved/)).toBeVisible();
  await expect(vendor).toHaveValue('');
  // The bill is now open for the vendor: the Vendor Center shows the balance.
  await page.getByRole('navigation', { name: 'Navigation' }).getByRole('button', { name: 'Vendors' }).click();
  await page.getByRole('textbox', { name: 'Find a vendor' }).fill("Bob's");
  await expect(page.getByTestId('center-vendors').getByRole('heading', { name: "Bob's Finishing" })).toBeVisible();
});

test('a credit card charge flips to a credit; a purchase order says it posts nothing', async ({ page }) => {
  await start(page, '/c/00000000-0000-4000-8000-00000000c0de/forms/cc-charge/new');
  const form = page.getByTestId('form-cc-charge');
  const card = form.getByRole('combobox', { name: 'Credit card account' });
  await card.click();
  await card.pressSequentially('Vis');
  await expect(card).toHaveValue('Visa');
  await page.keyboard.press('Tab');
  // The segmented control's radio input is visually hidden; its label is what people click.
  await form.getByText('Refund/Credit', { exact: true }).click();
  await expect(page.getByTestId('form-cc-credit')).toBeVisible();
  await page.goto('/c/00000000-0000-4000-8000-00000000c0de/forms/purchase-order/new');
  const po = page.getByTestId('form-purchase-order');
  await expect(po.getByText('Non-posting')).toBeVisible();
  await expect(po.getByRole('tab', { name: /Expenses/ })).toHaveCount(0);
});

test('create an invoice: the customer brings terms, tax follows the lines, the server confirms the total', async ({ page }) => {
  await start(page);
  await page.keyboard.press('Control+I');
  const form = page.getByTestId('form-invoice');
  await expect(form).toBeVisible();
  const customer = form.getByRole('combobox', { name: 'Customer:Job' });
  await expect(customer).toBeFocused();
  await customer.pressSequentially('Acme');
  await expect(customer).toHaveValue('Acme Corp');
  await page.keyboard.press('Tab');
  await expect(form.getByRole('combobox', { name: 'Terms' })).toHaveValue('Net 30');
  await expect(form.getByRole('textbox', { name: 'Due date' })).not.toHaveValue('');
  const item = form.getByRole('combobox', { name: 'Item, line 1' });
  await item.click();
  await item.pressSequentially('Walnut');
  await page.keyboard.press('Tab');
  await expect(form.getByRole('textbox', { name: 'Description, line 1' })).toHaveValue(/walnut/i);
  const qty = form.getByRole('textbox', { name: 'Qty, line 1' });
  await qty.fill('10');
  await page.keyboard.press('Tab');
  await expect(form.getByRole('textbox', { name: 'Amount, line 1' })).toHaveValue('185.00');
  // Walnut is taxable at 7.25%: 13.41 tax, 198.41 total.
  const totals = form.getByTestId('sales-totals');
  await expect(totals.getByLabel('Total')).toHaveText('198.41');
  await form.getByRole('checkbox', { name: 'Show postings' }).check();
  await expect(form.getByRole('cell', { name: 'Accounts Receivable' })).toBeVisible();
  await form.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText(/Invoice \d+ for \$198\.41 saved/)).toBeVisible();
});

test('receive a payment (auto-applied oldest first), deposit it, and balance a journal entry', async ({ page }) => {
  await start(page, '/c/00000000-0000-4000-8000-00000000c0de/forms/receive-payment/new');
  // A customer with an open invoice, from the books themselves.
  const customer = await page.evaluate(async () => {
    const lb = (window as unknown as { __librebooks: { txns(co: string, q: object): Promise<Array<{ entity_name: string | null; open_balance: number | null }>> } }).__librebooks;
    const open = await lb.txns('00000000-0000-4000-8000-00000000c0de', { types: ['Invoice'], open_only: true });
    return open[0]!.entity_name!.split(':')[0]!;
  });
  const form = page.getByTestId('form-receive-payment');
  const from = form.getByRole('combobox', { name: 'Received from' });
  await from.pressSequentially(customer);
  await page.keyboard.press('Tab');
  const items = form.getByRole('table', { name: 'Open items' });
  await expect(items.getByRole('row')).not.toHaveCount(1);
  const amount = form.getByRole('textbox', { name: 'Amount', exact: true });
  await amount.fill('100');
  await page.keyboard.press('Tab');
  // Auto-apply: the oldest open invoice took the 100.00.
  await expect(items.getByRole('textbox', { name: /^Payment for/ }).first()).toHaveValue('100.00');
  await expect(form.getByTestId('payment-totals').getByLabel('Unapplied amount')).toHaveText('0.00');
  await form.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText(/Payment of \$100\.00 saved/)).toBeVisible();

  // Make Deposits offers what's waiting in Undeposited Funds; our 100.00 is there.
  // (In-app navigation: a page load would reset the in-memory fixture books.)
  await page.getByRole('menubar').getByText('Banking', { exact: true }).click();
  await page.getByRole('menuitem', { name: /Make Deposits/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Payments to Deposit' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'OK' }).click();
  const deposit = page.getByTestId('form-deposit');
  await expect(deposit.getByTestId('deposit-totals').getByLabel('Deposit total')).not.toHaveText('0.00');
  await deposit.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText(/Deposit of \$[\d,]+\.\d\d saved/)).toBeVisible();

  // A journal entry prefills the balancing line.
  await page.getByRole('menubar').getByText('Company', { exact: true }).click();
  await page.getByRole('menuitem', { name: /Make General Journal Entries/ }).click();
  const journal = page.getByTestId('form-journal');
  const acct = journal.getByRole('combobox', { name: 'Account, line 1' });
  await acct.click();
  await acct.pressSequentially('Office Sup');
  await page.keyboard.press('Tab');
  await journal.getByRole('textbox', { name: 'Debit, line 1' }).fill('25');
  await page.keyboard.press('Tab');
  await page.keyboard.press('Tab');
  await journal.getByRole('textbox', { name: 'Memo, line 1' }).fill('Supplies');
  const acct2 = journal.getByRole('combobox', { name: 'Account, line 2' });
  await acct2.click();
  await expect(journal.getByRole('textbox', { name: 'Credit, line 2' })).toHaveValue('25.00');
  await expect(journal.getByTestId('journal-difference')).toHaveText('Balanced');
  await acct2.pressSequentially('Checking');
  await page.keyboard.press('Tab');
  await journal.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText(/Journal entry .*for \$25\.00 saved/)).toBeVisible();
});

test('a register line edits in place with Ctrl+E; 1-Line mode; Ctrl+Q QuickZooms', async ({ page }) => {
  const checking = '00000000-0000-4000-8000-000000000001';
  await start(page, `/c/00000000-0000-4000-8000-00000000c0de/accounts/${checking}/register`);
  const grid = page.getByTestId('register-grid');
  // This month's rent check: one expense line, so it edits in place (a split opens its form).
  await grid.getByText('Shop rent').last().click();
  await page.keyboard.press('Control+E');
  const editor = grid.locator('[data-editing]');
  await expect(editor).toBeVisible();
  const memo = editor.getByRole('textbox', { name: 'Memo' });
  await memo.fill('edited in the register');
  await page.keyboard.press('Enter');
  await expect(page.getByText('Saved.')).toBeVisible();
  await expect(editor).toBeHidden();
  await expect(grid.getByText('edited in the register')).toBeVisible();
  // 1-Line mode shows the memo on the same row as the date.
  await page.getByRole('checkbox', { name: '1-Line' }).check();
  await expect(grid.locator('[data-selected] [class*="one"]')).toHaveCount(1);
  // Ctrl+Q opens the selected transaction's form.
  await grid.focus();
  await page.keyboard.press('Control+Q');
  await expect(page.getByRole('tab', { name: /Check \d+/ })).toBeVisible();
});

test('reconcile: begin, mark all deposits, the difference moves, discard', async ({ page }) => {
  const checking = '00000000-0000-4000-8000-000000000001';
  await start(page, `/c/00000000-0000-4000-8000-00000000c0de/accounts/${checking}/reconcile`);
  const begin = page.getByTestId('reconcile-begin');
  await expect(begin.getByTestId('beginning-balance')).toBeVisible();
  const ending = begin.getByRole('textbox', { name: 'Ending balance' });
  await ending.fill('5000');
  await page.keyboard.press('Tab');
  await begin.getByRole('button', { name: 'Continue' }).click();
  const rec = page.getByTestId('reconcile');
  await expect(rec.getByTestId('reconcile-payments')).toBeVisible();
  await expect(rec.getByTestId('reconcile-deposits')).toBeVisible();
  const diffBefore = await rec.getByTestId('reconcile-summary').getByLabel('Difference').textContent();
  await rec.getByTestId('reconcile-deposits').getByRole('button', { name: 'Mark All', exact: true }).click();
  await expect(rec.getByTestId('reconcile-deposits').getByRole('checkbox').first()).toBeChecked();
  await expect(rec.getByTestId('reconcile-summary').getByLabel('Difference')).not.toHaveText(diffBefore!);
  page.once('dialog', (d) => void d.accept());
  await rec.getByRole('button', { name: 'Discard' }).click();
  await expect(page.getByTestId('reconcile-begin')).toBeVisible();
});

test('bank feeds: review a download, match it to its register twin, add another from its rule', async ({ page }) => {
  await start(page);
  await page.getByRole('menubar').getByText('Banking', { exact: true }).click();
  await page.getByRole('menuitem', { name: /Bank Feeds/ }).click();
  const feeds = page.getByTestId('bank-feeds');
  await expect(feeds.getByTestId('feed-account-row').first()).toBeVisible();
  await feeds.getByRole('button', { name: 'Review' }).first().click();
  const rows = feeds.getByTestId('feed-row');
  await expect(rows.first()).toBeVisible();
  // A download that mirrors a register check (it carries the check number) has a likely match.
  const twin = rows.filter({ hasText: /CHECK \d+/ }).first();
  await twin.getByRole('button', { name: 'Match' }).click();
  const dialog = page.getByRole('dialog', { name: /Match to a transaction/ });
  await expect(dialog.getByTestId('feed-candidate').first()).toHaveAttribute('data-strong', 'true');
  await dialog.getByRole('button', { name: 'Match', exact: true }).first().click();
  await expect(page.getByText(/Matched to /)).toBeVisible();
  // The bank fee: its rule suggests the account; Add previews the postings and writes it.
  const fee = rows.filter({ hasText: 'MONTHLY SERVICE FEE' }).first();
  await fee.getByRole('button', { name: 'Add', exact: true }).click();
  const editor = feeds.getByTestId('feed-add-row');
  await expect(editor.getByRole('combobox', { name: 'Account' })).toHaveValue('Bank Service Charges');
  await editor.getByRole('button', { name: 'Preview' }).click();
  await expect(editor.getByRole('cell', { name: 'Bank Service Charges' })).toBeVisible();
  await editor.getByRole('button', { name: 'Add to register' }).click();
  await expect(page.getByText(/Added as Check/)).toBeVisible();
  // Rules are listed in order with their actions.
  await feeds.getByRole('tab', { name: 'Rules' }).click();
  await expect(feeds.getByTestId('feed-rule').first()).toContainText('Pacific Power');
});

test('saving an empty check shows the problem inline', async ({ page }) => {
  await start(page, '/c/00000000-0000-4000-8000-00000000c0de/checks/new');
  const form = page.getByTestId('write-checks');
  await form.getByRole('button', { name: /Save & Close/ }).click();
  await expect(form.getByRole('alert')).toContainText('Enter at least one expense or item line.');
});

test('QuickFill and the palette match any colon level', async ({ page }) => {
  await start(page);
  await page.keyboard.press('Control+K');
  await expect(page.getByPlaceholder(/Go to a form/)).toBeFocused();
  await page.keyboard.type('mat');
  await expect(page.getByText('Cost of Goods Sold:Materials')).toBeVisible();
  await page.keyboard.press('Escape');
});

test('the shortcut overlay lists the browser keys', async ({ page }) => {
  await start(page);
  await page.locator('body').click();
  await page.keyboard.press('Shift+?');
  const dialog = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('row', { name: /Write Checks/ })).toContainText('Alt+Shift+W');
  await expect(dialog.getByText('Date fields')).toBeVisible();
});

test('windows stay open as tabs and Ctrl+F6 cycles them, keeping unsaved edits', async ({ page }) => {
  await start(page);
  await page.keyboard.press('Alt+Shift+W');
  const memo = page.getByTestId('write-checks').getByRole('textbox', { name: 'Memo', exact: true });
  await memo.fill('half-written');
  await page.locator('body').click();
  await page.getByRole('menubar').getByText('Lists', { exact: true }).click();
  await page.getByRole('menuitem', { name: /Chart of Accounts/ }).click();
  await expect(page.getByTestId('chart-of-accounts')).toBeVisible();
  await expect(page.getByRole('tab', { name: /Write Checks/ })).toBeVisible();
  await page.keyboard.press('Control+F6'); // wraps to Home
  await page.keyboard.press('Control+F6'); // Write Checks
  await expect(memo).toBeVisible();
  await expect(memo).toHaveValue('half-written');
});

test('a report collapses and drills down to account detail, then a transaction', async ({ page }) => {
  await start(page, '/c/00000000-0000-4000-8000-00000000c0de/reports/ProfitAndLoss');
  const report = page.getByTestId('report-window');
  await report.getByRole('combobox', { name: 'Dates' }).click();
  await page.getByRole('option', { name: 'This Fiscal Year-to-date' }).click();
  const grid = report.getByTestId('report-grid');
  await expect(grid.getByText('Gross Profit')).toBeVisible();
  await grid.focus();
  await page.keyboard.press('End'); // the virtualized grid renders the bottom on demand
  await expect(grid.getByText('Net Income', { exact: true })).toBeVisible();
  await page.keyboard.press('Home');
  await grid.getByRole('button', { name: 'Collapse Income' }).click();
  await expect(grid.getByText('Total Income')).toBeHidden();
  await grid.getByRole('button', { name: 'Expand Income' }).click();
  // Options: monthly columns add a TOTAL column; % of income adds percent columns.
  await expect(page).toHaveURL(/dates=ThisFiscalYearToDate/);
  await report.getByRole('combobox', { name: 'Columns' }).click();
  await page.getByRole('option', { name: 'Month' }).click();
  await expect(grid.getByRole('columnheader', { name: 'TOTAL' })).toBeVisible();
  await report.getByRole('button', { name: 'Options' }).click();
  await page.getByRole('checkbox', { name: '% of income' }).click();
  await expect(grid.getByRole('columnheader', { name: '% of Income' }).first()).toBeVisible();
  await page.keyboard.press('Escape');
  await grid.getByText('Design Services', { exact: true }).dblclick();
  await expect(page.getByRole('tab', { name: /Transaction Detail by Account: Design Services/ })).toBeVisible();
  const detail = page.locator('[data-window^="report:TransactionDetailByAccount"]').getByTestId('report-grid');
  await detail.getByRole('row').nth(2).dblclick();
  await expect(page.getByRole('tab', { name: /Invoice/ })).toBeVisible();
});

test('the Report Center lists QBD categories; memorizing a report keeps it', async ({ page }) => {
  await start(page, '/c/00000000-0000-4000-8000-00000000c0de/reports');
  const center = page.getByTestId('report-center');
  for (const c of ['Company & Financial', 'Customers & Receivables', 'Vendors & Payables', 'Banking', 'Accountant & Taxes', 'Lists']) {
    await expect(center.getByText(c, { exact: true })).toBeVisible();
  }
  await center.getByRole('button', { name: 'Balance Sheet', exact: true }).click();
  const report = page.getByTestId('report-window');
  await expect(report.getByTestId('report-grid').getByText('TOTAL ASSETS')).toBeVisible();
  await expect(report.getByRole('textbox', { name: 'As of' })).toBeVisible();
  await report.getByRole('button', { name: 'Memorize report' }).click();
  const dialog = page.getByRole('dialog', { name: 'Memorize Report' });
  await dialog.getByRole('textbox', { name: 'Name' }).fill('Month-end balance sheet');
  await dialog.getByRole('button', { name: 'Memorize' }).click();
  await expect(page.getByRole('tab', { name: 'Month-end balance sheet' })).toBeVisible();
  await page.getByRole('tab', { name: 'Report Center' }).click();
  await expect(center.getByText('Memorized Reports')).toBeVisible();
  await expect(center.getByText('Month-end balance sheet')).toBeVisible();
});

test("someone else's save shows a banner, and saving over it asks", async ({ page }) => {
  await start(page);
  // Open the first rent check from the Checking register.
  const txnId = await page.evaluate(async () => {
    const lb = (window as unknown as { __librebooks: { txns(co: string, q: object): Promise<Array<{ id: string; txn_type: string; entity_name: string | null }>> } }).__librebooks;
    const list = await lb.txns('00000000-0000-4000-8000-00000000c0de', { types: ['Check'] });
    return list.find((t) => t.entity_name === 'Main Street Properties')!.id;
  });
  await page.goto(`/c/00000000-0000-4000-8000-00000000c0de/txns/${txnId}`);
  const form = page.getByTestId('write-checks');
  const memo = form.getByRole('textbox', { name: 'Memo', exact: true });
  await memo.fill('my edit');
  await page.evaluate((id) => (window as unknown as { __librebooks: { simulateRemoteEdit(id: string): boolean } }).__librebooks.simulateRemoteEdit(id), txnId);
  await expect(page.getByText('Someone else just saved a change to this transaction.')).toBeVisible();
  await form.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByRole('dialog', { name: 'Someone else changed this' })).toBeVisible();
  await page.getByRole('button', { name: 'Save mine anyway' }).click();
  await expect(page.getByText(/saved\./)).toBeVisible();
});

const CO = '/c/00000000-0000-4000-8000-00000000c0de';

test('enrolment links: mint one, shown once as text and a QR code', async ({ page }) => {
  await start(page, `${CO}/admin/enrollments`);
  const win = page.getByTestId('enrollments');
  await expect(win.getByTestId('enrollment-row')).toHaveCount(1);
  await win.getByRole('button', { name: 'New link' }).click();
  const dialog = page.getByRole('dialog', { name: 'New enrolment link' });
  await dialog.getByRole('textbox', { name: 'Their name' }).fill('Pat Quinn');
  await dialog.getByRole('button', { name: 'Mint link' }).click();
  const minted = page.getByTestId('minted-link');
  await expect(minted.getByRole('img', { name: 'Enrolment link as a QR code' })).toBeVisible();
  await expect(minted.getByRole('textbox', { name: 'Enrolment link' })).toHaveValue(/\/enroll#token=/);
  await minted.getByRole('button', { name: 'Done' }).click();
  await expect(win.getByTestId('enrollment-row')).toHaveCount(2);
  await expect(win.getByText('Pat Quinn')).toBeVisible();
});

test('users: change a role from the detail drawer; you cannot demote yourself', async ({ page }) => {
  await start(page, `${CO}/admin/users`);
  const win = page.getByTestId('users');
  await win.getByRole('row', { name: /Maya Okafor/ }).click();
  const detail = page.getByTestId('user-detail');
  await detail.getByRole('combobox', { name: 'Server role' }).click();
  await page.getByRole('option', { name: 'Admin' }).click();
  await detail.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Maya Okafor saved.')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(win.getByRole('row', { name: /Maya Okafor, Admin/ })).toBeVisible();
  await win.getByRole('row', { name: /Demo User/ }).click();
  await expect(page.getByTestId('user-detail').getByRole('combobox', { name: 'Server role' })).toBeDisabled();
});

test('server settings: a write-only secret, and the update that is available', async ({ page }) => {
  await start(page, `${CO}/admin/server`);
  const win = page.getByTestId('server-settings');
  await win.getByRole('tab', { name: 'Secrets' }).click();
  await expect(win.getByTestId('secret-row')).toHaveCount(1);
  await win.getByRole('textbox', { name: 'Name' }).fill('s3-secret-key');
  await win.getByLabel('Value').fill('hunter2');
  await win.getByRole('button', { name: 'Add secret' }).click();
  await expect(win.getByTestId('secret-row')).toHaveCount(2);
  await expect(win.getByText('hunter2')).toHaveCount(0);
  await win.getByRole('tab', { name: 'Updates' }).click();
  await expect(win.getByTestId('update-available')).toContainText('0.4.2');
  await win.getByRole('tab', { name: 'Tunnel' }).click();
  await expect(win.getByTestId('tunnel-state')).toHaveText('stopped');
});

test('closing date: set it from Company, then an old transaction refuses to save', async ({ page }) => {
  await start(page, `${CO}/company`);
  const card = page.getByTestId('closing-date');
  await card.getByRole('textbox', { name: 'Closing date' }).fill('12/31/2025');
  await page.keyboard.press('Tab');
  await card.getByRole('button', { name: 'Set' }).click();
  await expect(card.getByText(/closed through 12\/31\/2025/)).toBeVisible();
  // The opening journal entry is dated well inside the closed period.
  const txnId = await page.evaluate(async () => {
    const lb = (window as unknown as { __librebooks: { txns(co: string, q: object): Promise<Array<{ id: string; date: string }>> } }).__librebooks;
    const list = await lb.txns('00000000-0000-4000-8000-00000000c0de', { types: ['Check'] });
    return [...list].sort((a, b) => a.date.localeCompare(b.date))[0]!.id;
  });
  // Open the transaction window in-app (a reload would reset the fixture books).
  await page.evaluate((id) => history.pushState(null, '', `/c/00000000-0000-4000-8000-00000000c0de/txns/${id}`), txnId);
  await page.evaluate(() => dispatchEvent(new PopStateEvent('popstate')));
  const form = page.getByTestId('write-checks');
  await form.getByRole('textbox', { name: 'Memo', exact: true }).fill('late edit');
  await form.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText(/books are closed through/)).toBeVisible();
});

test('memorize a check with Ctrl+M, find it on the Memorized Transaction List, enter it now', async ({ page }) => {
  await start(page);
  await page.keyboard.press('Alt+Shift+W');
  const form = page.getByTestId('write-checks');
  const payee = form.getByRole('combobox', { name: 'Payee' });
  await payee.pressSequentially('Office D');
  await page.keyboard.press('Tab');
  const amount = form.getByRole('textbox', { name: 'Check amount' });
  await amount.fill('125');
  await page.keyboard.press('Tab');
  await page.keyboard.press('Control+M');
  const dialog = page.getByRole('dialog', { name: 'Memorize Transaction' });
  await expect(dialog.getByRole('textbox', { name: 'Name' })).toHaveValue('Office Depot');
  await dialog.getByRole('button', { name: 'Memorize' }).click();
  await expect(page.getByText('Memorized "Office Depot".')).toBeVisible();
  await page.keyboard.press('Alt+Shift+T');
  const list = page.getByTestId('memorized-list');
  const row = list.getByTestId('memorized-row').filter({ hasText: 'Office Depot' });
  await expect(row).toBeVisible();
  await expect(row).toContainText('125.00');
  await row.getByRole('button', { name: 'Enter now' }).click();
  const entered = page.getByTestId('write-checks').last();
  await expect(entered.getByRole('combobox', { name: 'Payee' })).toHaveValue('Office Depot');
  await expect(entered.getByRole('textbox', { name: 'Check amount' })).toHaveValue('125.00');
});

test('attachments: upload to the inbox, then attach to a transaction', async ({ page }) => {
  await start(page, `${CO}/attachments`);
  const win = page.getByTestId('attachments');
  await expect(win.getByTestId('attachment-row')).toHaveCount(2);
  await win.locator('input[type=file]:not([aria-label])').setInputFiles({ name: 'receipt.txt', mimeType: 'text/plain', buffer: Buffer.from('Lumber $54.00') });
  // Notifications render in their own corner, outside the window.
  await expect(page.getByText('Added receipt.txt to the inbox.')).toBeVisible();
  await expect(win.getByTestId('attachment-row')).toHaveCount(3);
  await win.getByTestId('attachment-row').filter({ hasText: 'receipt.txt' }).getByRole('button', { name: 'Attach to…' }).click();
  const dialog = page.getByRole('dialog', { name: /Attach "receipt.txt"/ });
  await dialog.getByRole('textbox', { name: 'Find a transaction' }).fill('Office Depot');
  await expect(dialog.getByTestId('attach-target').first()).toContainText('Office Depot');
  await dialog.getByTestId('attach-target').first().click();
  await expect(page.getByText('Attached.')).toBeVisible();
  await expect(win.getByTestId('attachment-row')).toHaveCount(2);
  // The Attached tab lists what has found its record, with a way into it.
  await win.getByRole('tab', { name: 'Attached' }).click();
  await expect(win.getByTestId('attachment-row')).toHaveCount(1);
  await expect(win.getByTestId('attachment-row')).toContainText('receipt.txt');
  await expect(win.getByRole('button', { name: 'Open transaction' })).toBeVisible();
  await win.getByRole('button', { name: 'Detach' }).click();
  await expect(page.getByText('Moved back to the inbox.')).toBeVisible();
  await expect(win.getByTestId('attachment-row')).toHaveCount(0);
  await win.getByRole('tab', { name: 'Inbox' }).click();
  await expect(win.getByTestId('attachment-row')).toHaveCount(3);
  // Any record kind: attach the same file to a vendor, then follow it into the vendor's editor and back with its paperclip.
  await win.getByTestId('attachment-row').filter({ hasText: 'receipt.txt' }).getByRole('button', { name: 'Attach to…' }).click();
  const toName = page.getByRole('dialog', { name: /Attach "receipt.txt"/ });
  await toName.getByText('Name', { exact: true }).click();
  const who = toName.getByRole('combobox', { name: 'Attach to name' });
  await who.pressSequentially('Office D');
  await page.keyboard.press('Tab');
  await expect(who).toHaveValue('Office Depot');
  await toName.getByRole('button', { name: 'Attach', exact: true }).click();
  await expect(page.getByText('Attached.').last()).toBeVisible();
  await win.getByRole('tab', { name: 'Attached' }).click();
  await win.getByRole('button', { name: 'Open name: Office Depot' }).click();
  await expect(page.getByRole('tab', { name: 'Edit Vendor: Office Depot' })).toBeVisible();
  await page.getByTestId('record-vendor').getByRole('button', { name: 'Attachments' }).click();
  await expect(page.getByRole('tab', { name: 'Attachments: Office Depot' })).toBeVisible();
  const mine = page.locator('[data-window^="attachments:entity:"]');
  await expect(mine.getByText('On this name')).toBeVisible();
  await expect(mine.getByTestId('attachment-row').filter({ hasText: 'receipt.txt' }).first()).toBeVisible();
});

test('the Customer Center shows Balance and Balance Total: a job carries its invoice, the customer rolls it up', async ({ page }) => {
  await start(page, `${CO}/customers`);
  const list = page.getByRole('grid', { name: 'Customer list' });
  await expect(list.getByRole('columnheader', { name: 'Balance', exact: true })).toBeVisible();
  await expect(list.getByRole('columnheader', { name: 'Balance Total' })).toBeVisible();
  const cells = (name: string) => list.getByRole('row').filter({ has: page.getByRole('gridcell', { name, exact: true }) }).getByRole('gridcell');
  // Thompson Dental has no invoices of its own; its Reception Desk job has an open one.
  await expect(cells('Thompson Dental').nth(1)).toHaveText('');
  const total = await cells('Thompson Dental').nth(2).innerText();
  expect(total).toMatch(/\d/);
  await expect(cells('Reception Desk').nth(1)).toHaveText(total);
  await expect(cells('Reception Desk').nth(2)).toHaveText(total);
  await list.getByRole('gridcell', { name: 'Thompson Dental', exact: true }).click();
  const card = page.getByTestId('name-balance');
  await expect(card).toContainText('$0.00');
  await expect(card).toContainText(total);
  await expect(card).toContainText('with jobs');
});

test('a customer is created and edited in its own window from the Customer Center', async ({ page }) => {
  await start(page, `${CO}/customers`);
  await page.getByRole('button', { name: 'New Customer' }).click();
  const editor = page.getByTestId('record-customer');
  await expect(editor).toBeVisible();
  await expect(page.getByRole('tab', { name: 'New Customer' })).toBeVisible();
  const name = editor.getByRole('textbox', { name: 'Customer name' });
  await expect(name).toBeFocused();
  await name.fill('Pine Ridge Cabinets');
  // Enter moves on to the next field, as in QBD's dialogs.
  await page.keyboard.press('Enter');
  await expect(editor.getByRole('combobox', { name: 'Job of' })).toBeFocused();
  await editor.getByRole('textbox', { name: 'Company name' }).fill('Pine Ridge Cabinets LLC');
  await editor.getByRole('tab', { name: 'Payment Settings' }).click();
  await editor.getByRole('combobox', { name: 'Payment terms' }).click();
  await page.getByRole('option', { name: 'Net 15' }).click();
  await page.keyboard.press('Alt+A'); // Save & Close
  await expect(page.getByText('Customer "Pine Ridge Cabinets" saved.')).toBeVisible();
  await expect(editor).toBeHidden();
  // Back in the Customer Center: the new name is there; Ctrl+E edits it.
  await page.getByRole('textbox', { name: 'Find a customer' }).fill('Pine Ridge');
  const center = page.getByTestId('center-customers');
  await expect(center.getByRole('heading', { name: 'Pine Ridge Cabinets' })).toBeVisible();
  await expect(center.getByText('Pine Ridge Cabinets LLC')).toBeVisible();
  await page.keyboard.press('Control+E');
  await expect(page.getByRole('tab', { name: 'Edit Customer: Pine Ridge Cabinets' })).toBeVisible();
  const edit = page.getByTestId('record-customer');
  await edit.getByRole('textbox', { name: 'Main phone' }).fill('(555) 010-2020');
  await edit.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText('Customer "Pine Ridge Cabinets" saved.').last()).toBeVisible();
  await expect(center.getByText('(555) 010-2020')).toBeVisible();
  // Add Job opens a job editor under this customer.
  await center.getByRole('button', { name: 'Add job' }).click();
  const job = page.getByTestId('record-customer');
  await expect(job.getByRole('combobox', { name: 'Job of' })).toHaveValue('Pine Ridge Cabinets');
  await expect(page.getByRole('tab', { name: 'New Job' })).toBeVisible();
});

test('an item is created and edited from the Item List; a bad field is refused inline', async ({ page }) => {
  await start(page, `${CO}/lists/items`);
  const list = page.getByTestId('item-list');
  await expect(list).toBeVisible();
  await page.keyboard.press('Alt+Shift+N'); // browser keymap: New
  const editor = page.getByTestId('record-item');
  await expect(editor).toBeVisible();
  await editor.getByRole('combobox', { name: 'Item type' }).click();
  await page.getByRole('option', { name: 'Non-inventory Part' }).click();
  await editor.getByRole('textbox', { name: 'Item name' }).fill('Cherry Board Foot');
  await editor.getByRole('textbox', { name: 'Sales price' }).fill('twelve');
  await editor.getByRole('button', { name: /Save & Close/ }).click();
  await expect(editor.getByText('Not a number')).toBeVisible();
  await expect(editor.getByText('Choose an account.')).toBeVisible();
  await editor.getByRole('textbox', { name: 'Sales price' }).fill('12.50');
  const account = editor.getByRole('combobox', { name: 'Account' });
  await account.click();
  await account.pressSequentially('Design');
  await page.keyboard.press('Tab');
  await expect(account).toHaveValue('Design Services');
  await editor.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText('Item "Cherry Board Foot" saved.')).toBeVisible();
  // The list has it; Ctrl+E opens the editor on it.
  await list.getByRole('textbox', { name: 'Find' }).fill('Cherry');
  await expect(list.getByText('Cherry Board Foot')).toBeVisible();
  await list.getByText('Cherry Board Foot').click();
  await page.keyboard.press('Control+E');
  await expect(page.getByRole('tab', { name: 'Edit Item: Cherry Board Foot' })).toBeVisible();
  const edit = page.getByTestId('record-item');
  await expect(edit.getByRole('combobox', { name: 'Item type' })).toBeDisabled();
  await edit.getByRole('textbox', { name: 'Description' }).fill('Cherry lumber, per board foot');
  await page.keyboard.press('Alt+A');
  await expect(page.getByText('Item "Cherry Board Foot" saved.').last()).toBeVisible();
  await expect(list.getByText('Cherry lumber, per board foot')).toBeVisible();
});

test('deleting an account in use is refused with the reason; terms are edited in their window', async ({ page }) => {
  await start(page, `${CO}/accounts`);
  const chart = page.getByTestId('chart-of-accounts');
  await chart.getByRole('textbox', { name: 'Find an account' }).fill('Checking');
  await page.keyboard.press('Control+E');
  const editor = page.getByTestId('record-account');
  await expect(page.getByRole('tab', { name: 'Edit Account: Checking' })).toBeVisible();
  page.once('dialog', (d) => void d.accept());
  await editor.getByRole('button', { name: /Delete/ }).click();
  await expect(editor.getByRole('alert')).toContainText('make it inactive instead');
  await page.keyboard.press('Escape');
  // Terms: date-driven fields appear with the kind.
  await page.getByRole('menubar').getByText('Lists', { exact: true }).click();
  await page.getByRole('menuitem', { name: /Terms List/ }).click();
  const terms = page.getByTestId('terms-list');
  await expect(terms.getByText('2% if paid in 10 days')).toBeVisible();
  await terms.getByRole('button', { name: 'New' }).click();
  const t = page.getByTestId('record-terms');
  await t.getByRole('textbox', { name: 'Terms name' }).fill('1st of month');
  await t.getByText('Date driven', { exact: true }).click();
  await expect(t.getByRole('textbox', { name: 'Net due before the (day of month)' })).toHaveValue('1');
  await t.getByRole('button', { name: /Save & Close/ }).click();
  await expect(page.getByText('Terms "1st of month" saved.')).toBeVisible();
  await expect(terms.getByText('Due on the 1st')).toBeVisible();
});

test('the company picker: a new company from its information, and a .lbk file, each opens as a new company', async ({ page }) => {
  // `/companies` shows the list even when the one company would open by itself.
  await page.goto('/companies?fixtures=1&latency=0&keymap=browser');
  const picker = page.getByTestId('company-picker');
  await expect(picker.getByRole('button', { name: 'Juniper Hill Woodworks' })).toBeVisible();
  await picker.getByRole('button', { name: 'New company…' }).click();
  const dialog = page.getByRole('dialog', { name: 'New company' });
  await expect(dialog.getByRole('button', { name: 'Create company' })).toBeDisabled();
  await dialog.getByRole('textbox', { name: 'Company name' }).fill('Second Shop');
  await dialog.getByRole('combobox', { name: 'Fiscal year starts' }).click();
  await page.getByRole('option', { name: 'July' }).click();
  await dialog.getByRole('button', { name: 'Create company' }).click();
  await expect(page).toHaveURL(/\/c\/[0-9a-f-]+\/home/);
  await expect(page).toHaveTitle(/Second Shop/);
  await expect(page.getByText('"Second Shop" is ready.')).toBeVisible();
  // Company → Company Information shows what was entered.
  await page.getByRole('menubar').getByText('Company', { exact: true }).click();
  await page.getByRole('menuitem', { name: 'Company Information' }).click();
  await expect(page.getByTestId('company').getByText('July')).toBeVisible();
  // The user menu leads back to the list, which now has both; a .lbk file opens as a third.
  await page.getByRole('button', { name: 'Signed in as Demo User' }).click();
  await page.getByRole('menuitem', { name: 'Open another company' }).click();
  await expect(picker.getByRole('button', { name: 'Second Shop' })).toBeVisible();
  await picker.getByRole('button', { name: 'Open a .lbk file…' }).click();
  const open = page.getByRole('dialog', { name: 'Open a company file' });
  await expect(open.getByRole('button', { name: 'Open as a new company' })).toBeDisabled();
  await open.locator('input[type=file]').setInputFiles({ name: 'riverside-cafe.lbk', mimeType: 'application/zip', buffer: Buffer.from('not really a zip, which the fixture accepts') });
  await open.getByRole('button', { name: 'Open as a new company' }).click();
  await expect(page).toHaveTitle(/riverside-cafe/);
  await expect(page.getByText(/"riverside-cafe" is a new company/)).toBeVisible();
});

test('QuickBooks import: a sync runs with progress and lands a report; parity compares balances', async ({ page }) => {
  await start(page, `${CO}/import/qbd`);
  const win = page.getByTestId('qbd-import');
  await expect(win.getByTestId('sync-report')).toContainText('Last sync');
  await win.getByRole('button', { name: 'Full re-sync' }).click();
  await expect(win.getByTestId('sync-progress')).toBeVisible();
  await expect(win.getByTestId('sync-progress')).toBeHidden({ timeout: 15_000 });
  await expect(win.getByTestId('sync-report')).toContainText('Last sync (full)');
  await win.getByRole('tab', { name: 'Parity' }).click();
  await win.getByRole('button', { name: 'Compare now' }).click();
  const table = win.getByTestId('parity-table');
  await expect(table.locator('tr[data-off]')).toHaveCount(2);
  await expect(win.getByText(/2 off by/)).toBeVisible();
  await win.getByRole('tab', { name: 'Connection' }).click();
  await expect(win.getByRole('textbox', { name: 'Bridge URL' })).toHaveValue(/8750/);
});

test('merge folds one vendor into another', async ({ page }) => {
  await start(page, `${CO}/merge`);
  const win = page.getByTestId('merge');
  const from = win.getByRole('combobox', { name: 'Record to merge away' });
  await from.pressSequentially('Office D');
  await page.keyboard.press('Tab');
  await expect(from).toHaveValue('Office Depot');
  const into = win.getByRole('combobox', { name: 'Record to keep' });
  await into.pressSequentially('Main Street');
  await page.keyboard.press('Tab');
  await expect(into).toHaveValue('Main Street Properties');
  await win.getByRole('button', { name: 'Merge…' }).click();
  await page.getByRole('dialog', { name: 'Merge these two?' }).getByRole('button', { name: 'Merge', exact: true }).click();
  await expect(page.getByText('Merged "Office Depot" into "Main Street Properties".')).toBeVisible();
});

test('my account from the avatar menu: passkeys and sessions', async ({ page }) => {
  await start(page);
  await page.getByRole('button', { name: 'Signed in as Demo User' }).click();
  await page.getByRole('menuitem', { name: 'My Account' }).click();
  const win = page.getByTestId('my-account');
  await expect(win.getByTestId('passkey-row')).toHaveCount(2);
  await expect(win.getByTestId('session-row')).toHaveCount(3);
  await expect(win.getByText('This device')).toBeVisible();
  await win.getByRole('button', { name: 'Rename passkey YubiKey 5C' }).click();
  await page.getByRole('dialog', { name: 'Rename passkey' }).getByRole('textbox', { name: 'Name' }).fill('Blue YubiKey');
  await page.getByRole('dialog', { name: 'Rename passkey' }).getByRole('button', { name: 'Save' }).click();
  await expect(win.getByText('Blue YubiKey')).toBeVisible();
});

test('stress: a 100,000-line register and 5,000 more names stay responsive', async ({ page }) => {
  const stressAccount = '00000000-0000-4000-9fff-000000000001';
  await page.goto(`/c/00000000-0000-4000-8000-00000000c0de/accounts/${stressAccount}/register?fixtures=stress&latency=0&keymap=browser`);
  await expect(page.getByText('100,000 transactions')).toBeVisible({ timeout: 20_000 });
  const grid = page.getByTestId('register-grid');
  // Only a screenful of rows is in the DOM, however long the register.
  expect(await grid.getByRole('row').count()).toBeLessThan(80);
  await grid.focus();
  const t0 = Date.now();
  await page.keyboard.press('Control+Home');
  await expect(grid.locator('#reg-row-0')).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('Control+End'); // the new-transaction line, after 100,000 others
  await expect(grid.locator('#reg-row-100000[data-entry]')).toBeVisible();
  expect(Date.now() - t0).toBeLessThan(3_000);
  // QuickFill over every name (5,000+), filtering per keystroke.
  await page.goto('/c/00000000-0000-4000-8000-00000000c0de/checks/new');
  const payee = page.getByTestId('write-checks').getByRole('combobox', { name: 'Payee' });
  await payee.click();
  await payee.pressSequentially('juniper fre');
  await expect(page.getByRole('option', { name: /Juniper Freight/ }).first()).toBeVisible();
});
