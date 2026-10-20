import { expect, test, type Page } from '@playwright/test';

const CO = '/c/00000000-0000-4000-8000-00000000c0de';

async function start(page: Page, path = '/') {
  await page.goto(`${path}?fixtures=1&latency=0`);
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
}

/** Nothing on the page may be wider than the phone. */
async function expectNoSideways(page: Page) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
}

test('home at 375px: bottom tabs, no sideways scrolling', async ({ page }) => {
  await start(page);
  await expect(page.getByText('Bank and credit card accounts')).toBeVisible();
  for (const label of ['Home', 'Customers', 'Banking', 'More']) {
    await expect(page.getByRole('navigation', { name: 'Main' }).getByText(label, { exact: true })).toBeVisible();
  }
  await expectNoSideways(page);
  await page.screenshot({ path: test.info().outputPath('home-375.png'), fullPage: true });
});

test('the + sheet starts a check; lines are cards', async ({ page }) => {
  await start(page);
  await page.getByRole('button', { name: 'Create' }).tap();
  await page.getByText('Write Check', { exact: true }).tap();
  const form = page.getByTestId('write-checks');
  await expect(form).toBeVisible();
  const expenses = form.getByRole('tabpanel', { name: 'Expenses' });
  await expect(expenses.getByText('Line 1')).toBeVisible();
  await expect(expenses.getByRole('button', { name: 'Add line' })).toBeVisible();
  await expect(page.getByRole('button', { name: /Save & Close/ })).toBeVisible();
  await expectNoSideways(page);
  await page.screenshot({ path: test.info().outputPath('check-375.png') });
  // Back closes the window and returns to Home.
  await page.getByRole('button', { name: 'Back' }).tap();
  await expect(page.getByText('Bank and credit card accounts')).toBeVisible();
});

test('the + sheet starts a bill; the header stacks and the lines are cards', async ({ page }) => {
  await start(page);
  await page.getByRole('button', { name: 'Create' }).tap();
  await page.getByText('Enter Bill', { exact: true }).tap();
  const form = page.getByTestId('form-bill');
  await expect(form).toBeVisible();
  await expect(form.getByRole('combobox', { name: 'Vendor' })).toBeVisible();
  await expect(form.getByRole('textbox', { name: 'Bill due' })).toBeVisible();
  await expect(form.getByRole('tabpanel', { name: 'Expenses' }).getByText('Line 1')).toBeVisible();
  await expect(page.getByRole('button', { name: /Save & Close/ })).toBeVisible();
  await expectNoSideways(page);
  await page.screenshot({ path: test.info().outputPath('bill-375.png') });
});

test('banking → a register as a tappable list', async ({ page }) => {
  await start(page);
  await page.getByRole('navigation', { name: 'Main' }).getByText('Banking', { exact: true }).tap();
  await page.locator('[data-window="banking"]').getByText('Checking', { exact: true }).tap();
  const list = page.getByTestId('register-list');
  await expect(list).toBeVisible();
  await expect(list.getByRole('button', { name: /cleared/i }).first()).toBeVisible();
  await expectNoSideways(page);
  await page.screenshot({ path: test.info().outputPath('register-375.png') });
});

test('customers list → detail with transactions', async ({ page }) => {
  await start(page, `${CO}/customers`);
  await page.getByText('Acme Corp', { exact: true }).tap();
  await expect(page.getByRole('heading', { name: 'Acme Corp' })).toBeVisible();
  await expect(page.getByText('Transactions')).toBeVisible();
  await expectNoSideways(page);
  await page.screenshot({ path: test.info().outputPath('customer-375.png') });
  // The header's Back steps back to the list inside the window, then closes it.
  await page.getByRole('button', { name: 'Back' }).tap();
  await expect(page.getByRole('textbox', { name: 'Find a customer' })).toBeVisible();
  await page.getByRole('button', { name: 'Back' }).tap();
  await expect(page.getByText('Bank and credit card accounts')).toBeVisible();
});

test('a report scrolls sideways inside itself, not the page', async ({ page }) => {
  await start(page, `${CO}/reports/ProfitAndLoss`);
  await page.getByRole('combobox', { name: 'Dates' }).tap();
  await page.getByRole('option', { name: 'This Fiscal Year-to-date' }).tap();
  await expect(page.getByRole('combobox', { name: 'Dates' })).toHaveValue('This Fiscal Year-to-date');
  await expect(page.getByTestId('report-grid').getByText('Gross Profit')).toBeVisible();
  // The figures are on screen at 375px, not pushed off to the right.
  const amount = page.getByTestId('report-grid').getByRole('columnheader').nth(1);
  const box = await amount.boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(375);
  await expectNoSideways(page);
  await page.screenshot({ path: test.info().outputPath('report-375.png') });
});
