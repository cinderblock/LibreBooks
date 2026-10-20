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
