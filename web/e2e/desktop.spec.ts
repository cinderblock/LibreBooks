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

test('a report collapses and drills down to a QuickReport, then a transaction', async ({ page }) => {
  await start(page, '/c/00000000-0000-4000-8000-00000000c0de/reports/profit_and_loss');
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
  await grid.getByText('Design Services', { exact: true }).dblclick();
  await expect(page.getByRole('tab', { name: /Design Services/ })).toBeVisible();
  const qr = page.locator('[data-window^="report:account_quickreport"]').getByTestId('report-grid');
  await qr.getByRole('row').nth(2).dblclick();
  await expect(page.getByRole('tab', { name: /Invoice/ })).toBeVisible();
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
