// Every keyboard command the app knows, with where it lives in the menus and the
// scope it works in. The keys themselves are in ./keymaps.ts (they differ by
// platform); this file is the vocabulary the registry, menus and the shortcut
// overlay share.

/**
 * Narrowest wins: a key bound in a grid beats the same key bound by the form
 * around it, which beats the window, which beats global.
 */
export type Scope = 'global' | 'window' | 'form' | 'grid' | 'field';

export const SCOPE_RANK: Record<Scope, number> = { global: 0, window: 1, form: 2, grid: 3, field: 4 };

export const SCOPE_LABEL: Record<Scope, string> = {
  global: 'Anywhere',
  window: 'In a window',
  form: 'On a form',
  grid: 'In a line grid or register',
  field: 'In a field',
};

export interface CommandDef {
  label: string;
  scope: Scope;
  /** Fire while focus is in a text field? Default: only combos with Ctrl/Alt/⌘. */
  inFields?: boolean;
}

export const COMMANDS = {
  // Global: navigation and forms (QBD's Ctrl+letter shortcuts).
  'palette.open': { label: 'Search / go to anything', scope: 'global' },
  'help.shortcuts': { label: 'Keyboard shortcuts', scope: 'global' },
  'nav.home': { label: 'Home', scope: 'global' },
  'form.writeChecks': { label: 'Write Checks', scope: 'global' },
  'form.invoice': { label: 'Create Invoices', scope: 'global' },
  // QBD binds no keys to these; they're commands so menus, the palette and a
  // future keymap editor can reach them.
  'form.bill': { label: 'Enter Bills', scope: 'global' },
  'form.payBills': { label: 'Pay Bills', scope: 'global' },
  'form.vendorCredit': { label: 'Enter Vendor Credits', scope: 'global' },
  'form.purchaseOrder': { label: 'Create Purchase Orders', scope: 'global' },
  'form.ccCharge': { label: 'Enter Credit Card Charges', scope: 'global' },
  'form.salesReceipt': { label: 'Enter Sales Receipts', scope: 'global' },
  'form.receivePayment': { label: 'Receive Payments', scope: 'global' },
  'form.creditMemo': { label: 'Create Credit Memos/Refunds', scope: 'global' },
  'form.estimate': { label: 'Create Estimates', scope: 'global' },
  'form.salesOrder': { label: 'Create Sales Orders', scope: 'global' },
  'nav.statementCharges': { label: 'Enter Statement Charges', scope: 'global' },
  'form.deposit': { label: 'Make Deposits', scope: 'global' },
  'form.transfer': { label: 'Transfer Funds', scope: 'global' },
  'form.journal': { label: 'Make General Journal Entries', scope: 'global' },
  'nav.reconcile': { label: 'Reconcile', scope: 'global' },
  'nav.bankFeeds': { label: 'Bank Feeds', scope: 'global' },
  'nav.customerCenter': { label: 'Customer Center', scope: 'global' },
  'nav.vendorCenter': { label: 'Vendor Center', scope: 'global' },
  'nav.useRegister': { label: 'Use Register', scope: 'global' },
  // Ctrl+A in a text field selects all; it opens the chart only outside fields.
  'nav.chartOfAccounts': { label: 'Chart of Accounts', scope: 'global', inFields: false },
  'nav.find': { label: 'Find', scope: 'global' },
  'nav.memorizedTxns': { label: 'Memorized Transaction List', scope: 'global' },
  'nav.reports': { label: 'Report Center', scope: 'global' },
  'theme.toggle': { label: 'Toggle dark mode', scope: 'global' },
  'window.next': { label: 'Next window', scope: 'global' },
  'window.prev': { label: 'Previous window', scope: 'global' },

  // Window: acts on what the window shows (a list's selection, a form's record).
  'window.close': { label: 'Close window (asks if unsaved)', scope: 'window', inFields: true },
  'record.new': { label: 'New', scope: 'window' },
  'record.edit': { label: 'Edit', scope: 'window' },
  'record.delete': { label: 'Delete', scope: 'window' },
  'record.history': { label: 'Transaction history', scope: 'window' },
  'report.quick': { label: 'QuickReport', scope: 'window' },
  'txn.memorize': { label: 'Memorize transaction', scope: 'window' },
  'report.memorize': { label: 'Memorize report', scope: 'window' },
  // Ctrl+E is Edit in a list or register; a report has nothing to edit, so it exports (the last format picked).
  'report.export': { label: 'Export report', scope: 'window' },
  // Ctrl+Y is redo inside a text field on Windows; leave it there.
  'txn.journal': { label: 'Transaction journal', scope: 'window', inFields: false },
  'register.goToOther': { label: 'Go to the other register', scope: 'window' },
  'record.print': { label: 'Print', scope: 'window' },
  'list.show': { label: 'Show the list for this field', scope: 'window' },

  // Form.
  'form.saveNew': { label: 'Save & New', scope: 'form' },
  'form.saveClose': { label: 'Save & Close', scope: 'form' },
  'txn.prev': { label: 'Previous transaction', scope: 'form', inFields: true },
  'txn.next': { label: 'Next transaction', scope: 'form', inFields: true },

  // Grid.
  'grid.insertLine': { label: 'Insert line', scope: 'grid' },
  'grid.deleteLine': { label: 'Delete line', scope: 'grid' },
} as const satisfies Record<string, CommandDef>;

export type CommandId = keyof typeof COMMANDS;

export const COMMAND_IDS = Object.keys(COMMANDS) as CommandId[];

export function commandDef(id: CommandId): CommandDef {
  return COMMANDS[id];
}

/**
 * Keys that fields handle themselves (not routed through the registry, but
 * listed in the shortcut overlay so they're discoverable).
 */
export const FIELD_KEYS: ReadonlyArray<{ where: string; keys: ReadonlyArray<[string, string]> }> = [
  {
    where: 'Date fields',
    keys: [
      ['+ / -', 'Next / previous day'],
      ['T', 'Today'],
      ['W / K', 'First / last day of the week'],
      ['M / H', 'First / last day of the month'],
      ['Y / R', 'First / last day of the year'],
      ['[ / ]', 'Back / forward a week'],
      ['Alt+↓', 'Open the calendar'],
    ],
  },
  {
    where: 'Amount fields',
    keys: [
      ['12*3+4 =', 'Calculate (also Enter or Tab)'],
      ['+ / - on a filled field', 'Add to / subtract from the amount'],
      ['(12.00)', 'A negative amount'],
      ['Esc', 'Undo the calculation'],
    ],
  },
  {
    where: 'Name, account and item fields (QuickFill)',
    keys: [
      ['Type any part', '"Mat" finds "Cost of Goods Sold:Materials"; "cost:mat" goes level by level'],
      ['↓ / ↑', 'Move through matches'],
      ['Alt+↓ / Ctrl+L', 'Open the full list'],
      ['Enter / Tab', 'Choose the highlighted match'],
      ['Esc', 'Close the list'],
    ],
  },
  {
    where: 'Line grids',
    keys: [
      ['Tab / Shift+Tab', 'Next / previous cell'],
      ['↑ / ↓', 'Same column, previous / next line'],
      ['← / →', 'Previous / next cell when the cell is selected'],
      ['Enter', 'Next cell (configurable: next line, or save)'],
      ['Esc', 'Undo the cell, then close'],
    ],
  },
  {
    where: 'Registers',
    keys: [
      ['↑ / ↓', 'Previous / next transaction'],
      ['Page Up / Page Down', 'A page up / down'],
      ['Ctrl+Home / Ctrl+End', 'First / last (the new-transaction line)'],
      ['Enter', 'Open the selected transaction'],
      ['Space', 'Toggle cleared'],
    ],
  },
];
