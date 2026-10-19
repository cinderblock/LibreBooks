// FIXTURE ONLY: a small, realistic, entirely synthetic company for fixture mode.
//
// "Juniper Hill Woodworks" is a custom furniture shop: bank, savings and petty
// cash, two credit cards, a loan, parent/sub accounts on both sides of the P&L,
// customers with jobs, vendors with default expense accounts, two-sided items,
// and about four months of transactions dated relative to `today` (so "This
// Month" reports always have something in them). Ids are deterministic so URLs
// and e2e tests are stable across reloads. No real data, ever.

import { blankAccount, blankEntity, blankItem, blankLine, blankTxn } from '../blank';
import { addDays, addMonths, firstOfMonth } from '~/lib/dates';
import type {
  Account,
  AccountType,
  Class,
  CompanyInfo,
  Entity,
  EntityKind,
  Item,
  ItemType,
  PaymentMethod,
  SalesRep,
  SalesTaxCode,
  Terms,
  Txn,
  TxnLine,
  TypeEntry,
} from '../types';
import type { AccountRole } from '../generated/AccountRole';
import { extend } from '~/lib/money';

export const FIXTURE_COMPANY_ID = '00000000-0000-4000-8000-00000000c0de';

export interface SeedBooks {
  info: CompanyInfo;
  accounts: Account[];
  entities: Entity[];
  items: Item[];
  classes: Class[];
  terms: Terms[];
  paymentMethods: PaymentMethod[];
  salesTaxCodes: SalesTaxCode[];
  salesReps: SalesRep[];
  types: TypeEntry[];
  /** Unposted: the fixture client posts them in order. */
  txns: Txn[];
}

/** Deterministic UUID-shaped ids: `00000000-0000-4000-8000-0000000003e9`. */
function makeIds() {
  let n = 0;
  return () => `00000000-0000-4000-8000-${(++n).toString(16).padStart(12, '0')}`;
}

/** Small deterministic PRNG (mulberry32) so amounts vary but never change. */
function rng(seed: number) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function buildSeed(today: string): SeedBooks {
  const id = makeIds();
  const rand = rng(20261019);
  const between = (lo: number, hi: number) => Math.round(lo + rand() * (hi - lo));

  // ---- Accounts ------------------------------------------------------------
  const accounts: Account[] = [];
  const acct = new Map<string, Account>();
  const addAccount = (
    number: string,
    fullName: string,
    type: AccountType,
    extra: Partial<Account> = {},
  ): Account => {
    const parts = fullName.split(':');
    const parent = parts.length > 1 ? acct.get(parts.slice(0, -1).join(':')) : undefined;
    const a: Account = { ...blankAccount(parts[parts.length - 1]!, type, parent), id: id(), number, edit_seq: 1n, ...extra };
    accounts.push(a);
    acct.set(fullName, a);
    return a;
  };
  const role = (r: AccountRole) => ({ role: r });
  addAccount('1000', 'Checking', 'Bank', { description: 'First Valley Bank checking', bank_account_number: '****4471' });
  addAccount('1010', 'Savings', 'Bank', { bank_account_number: '****8810' });
  addAccount('1020', 'Petty Cash', 'Bank');
  addAccount('1200', 'Accounts Receivable', 'AccountsReceivable', role('AccountsReceivable'));
  addAccount('1300', 'Undeposited Funds', 'OtherCurrentAsset', role('UndepositedFunds'));
  addAccount('1400', 'Inventory Asset', 'OtherCurrentAsset', role('InventoryAsset'));
  addAccount('1500', 'Prepaid Insurance', 'OtherCurrentAsset');
  addAccount('1700', 'Shop Equipment', 'FixedAsset');
  addAccount('1710', 'Shop Equipment:Original Cost', 'FixedAsset');
  addAccount('1720', 'Shop Equipment:Accumulated Depreciation', 'FixedAsset');
  addAccount('1750', 'Vehicles', 'FixedAsset');
  addAccount('2000', 'Accounts Payable', 'AccountsPayable', role('AccountsPayable'));
  addAccount('2100', 'Visa', 'CreditCard', { description: 'Business Visa ****4421' });
  addAccount('2110', 'American Express', 'CreditCard');
  addAccount('2200', 'Payroll Liabilities', 'OtherCurrentLiability', role('PayrollLiabilities'));
  addAccount('2300', 'Sales Tax Payable', 'OtherCurrentLiability', role('SalesTaxPayable'));
  addAccount('2700', 'Equipment Loan', 'LongTermLiability');
  addAccount('3000', 'Opening Balance Equity', 'Equity', role('OpeningBalanceEquity'));
  addAccount('3100', "Owner's Equity", 'Equity');
  addAccount('3110', "Owner's Equity:Owner Draws", 'Equity');
  addAccount('3120', "Owner's Equity:Owner Contributions", 'Equity');
  addAccount('3900', 'Retained Earnings', 'Equity', role('RetainedEarnings'));
  addAccount('4000', 'Sales', 'Income');
  addAccount('4010', 'Sales:Custom Furniture', 'Income');
  addAccount('4020', 'Sales:Cabinetry', 'Income');
  addAccount('4030', 'Sales:Repairs & Refinishing', 'Income');
  addAccount('4100', 'Design Services', 'Income');
  addAccount('4900', 'Delivery Income', 'Income');
  addAccount('5000', 'Cost of Goods Sold', 'CostOfGoodsSold', role('CostOfGoodsSold'));
  addAccount('5010', 'Cost of Goods Sold:Materials', 'CostOfGoodsSold');
  addAccount('5020', 'Cost of Goods Sold:Hardware', 'CostOfGoodsSold');
  addAccount('5030', 'Cost of Goods Sold:Subcontractors', 'CostOfGoodsSold');
  addAccount('6000', 'Advertising and Promotion', 'Expense');
  addAccount('6100', 'Automobile Expense', 'Expense');
  addAccount('6110', 'Automobile Expense:Fuel', 'Expense');
  addAccount('6120', 'Automobile Expense:Repairs & Maintenance', 'Expense');
  addAccount('6200', 'Bank Service Charges', 'Expense');
  addAccount('6300', 'Insurance Expense', 'Expense');
  addAccount('6310', 'Insurance Expense:Liability Insurance', 'Expense');
  addAccount('6320', "Insurance Expense:Workers' Compensation", 'Expense');
  addAccount('6400', 'Office Supplies', 'Expense');
  addAccount('6500', 'Rent Expense', 'Expense');
  addAccount('6600', 'Repairs and Maintenance', 'Expense');
  addAccount('6700', 'Tools and Small Equipment', 'Expense');
  addAccount('6800', 'Utilities', 'Expense');
  addAccount('6810', 'Utilities:Electric', 'Expense');
  addAccount('6820', 'Utilities:Internet & Phone', 'Expense');
  addAccount('6830', 'Utilities:Water & Sewer', 'Expense');
  addAccount('6900', 'Meals and Entertainment', 'Expense');
  addAccount('6950', 'Payroll Expenses', 'Expense', role('PayrollExpenses'));
  addAccount('6990', 'Ask My Accountant', 'Expense', role('AskMyAccountant'));
  addAccount('7000', 'Interest Income', 'OtherIncome');
  addAccount('8000', 'Interest Expense', 'OtherExpense');
  addAccount('8500', 'Old Equipment Rental', 'OtherExpense', { is_active: false });
  addAccount('9000', 'Estimates', 'NonPosting', role('Estimates'));
  addAccount('9010', 'Purchase Orders', 'NonPosting', role('PurchaseOrders'));
  const A = (name: string) => {
    const a = acct.get(name);
    if (!a) throw new Error(`seed: no account ${name}`);
    return a.id;
  };

  // ---- Small lists ---------------------------------------------------------
  const terms: Terms[] = [
    { id: id(), name: 'Net 30', is_active: true, kind: { type: 'Standard', due_days: 30, discount_days: null, discount_percent: null }, edit_seq: 1n },
    { id: id(), name: 'Net 15', is_active: true, kind: { type: 'Standard', due_days: 15, discount_days: null, discount_percent: null }, edit_seq: 1n },
    { id: id(), name: 'Due on receipt', is_active: true, kind: { type: 'Standard', due_days: 0, discount_days: null, discount_percent: null }, edit_seq: 1n },
    { id: id(), name: '2% 10 Net 30', is_active: true, kind: { type: 'Standard', due_days: 30, discount_days: 10, discount_percent: '2' }, edit_seq: 1n },
  ];
  const classes: Class[] = ['Shop', 'Field', 'Field:Installs'].map((full) => {
    const parts = full.split(':');
    return { id: id(), name: parts[parts.length - 1]!, full_name: full, parent_id: null, sublevel: parts.length - 1, is_active: true, edit_seq: 1n };
  });
  classes[2]!.parent_id = classes[1]!.id;
  const paymentMethods: PaymentMethod[] = (
    [
      ['Cash', 'Cash'],
      ['Check', 'Check'],
      ['Visa', 'Visa'],
      ['MasterCard', 'MasterCard'],
      ['ACH', 'ECheck'],
    ] as const
  ).map(([name, kind]) => ({ id: id(), name, kind, is_active: true, edit_seq: 1n }));
  const salesTaxCodes: SalesTaxCode[] = [
    { id: id(), name: 'Tax', description: 'Taxable sales', is_taxable: true, is_active: true, edit_seq: 1n },
    { id: id(), name: 'Non', description: 'Non-taxable sales', is_taxable: false, is_active: true, edit_seq: 1n },
  ];
  const types: TypeEntry[] = [
    { id: id(), list: 'CustomerType', name: 'Commercial', full_name: 'Commercial', parent_id: null, is_active: true, edit_seq: 1n },
    { id: id(), list: 'CustomerType', name: 'Residential', full_name: 'Residential', parent_id: null, is_active: true, edit_seq: 1n },
    { id: id(), list: 'VendorType', name: 'Supplier', full_name: 'Supplier', parent_id: null, is_active: true, edit_seq: 1n },
    { id: id(), list: 'JobType', name: 'New Build', full_name: 'New Build', parent_id: null, is_active: true, edit_seq: 1n },
  ];

  // ---- Names ---------------------------------------------------------------
  const entities: Entity[] = [];
  const ent = new Map<string, Entity>();
  const addEntity = (kind: EntityKind, fullName: string, extra: Partial<Entity> = {}): Entity => {
    const parts = fullName.split(':');
    const parent = parts.length > 1 ? ent.get(parts.slice(0, -1).join(':')) : undefined;
    const e: Entity = { ...blankEntity(parts[parts.length - 1]!, kind, parent), id: id(), edit_seq: 1n, ...extra };
    entities.push(e);
    ent.set(fullName, e);
    return e;
  };
  const addr = (line: string, city: string, state: string, zip: string) => ({
    lines: [line],
    city,
    state,
    postal_code: zip,
    country: null,
    note: null,
  });
  addEntity('Customer', 'Acme Corp', { company_name: 'Acme Corporation', phone: '(555) 201-4400', email: 'ap@acme.example', terms_id: terms[0]!.id, bill_address: addr('400 Commerce Way', 'Juniper', 'OR', '97000'), type_id: types[0]!.id });
  addEntity('Customer', 'Acme Corp:Lobby Bench', { job_status: 'InProgress', job_description: 'White oak lobby bench, 12 ft' });
  addEntity('Customer', 'Acme Corp:Conference Table', { job_status: 'Awarded' });
  addEntity('Customer', 'Baker, Ellen', { first_name: 'Ellen', last_name: 'Baker', phone: '(555) 310-2219', email: 'ellen.baker@example.com', terms_id: terms[2]!.id, bill_address: addr('18 Alder Lane', 'Juniper', 'OR', '97001'), type_id: types[1]!.id });
  addEntity('Customer', 'Baker, Ellen:Kitchen Cabinets', { job_status: 'InProgress' });
  addEntity('Customer', 'Riverside Cafe', { company_name: 'Riverside Cafe LLC', phone: '(555) 442-0187', terms_id: terms[1]!.id, bill_address: addr('2 River Rd', 'Juniper', 'OR', '97002') });
  addEntity('Customer', 'Riverside Cafe:Patio Furniture', { job_status: 'Closed' });
  addEntity('Customer', 'Thompson Dental', { company_name: 'Thompson Family Dental', terms_id: terms[0]!.id });
  addEntity('Customer', 'Thompson Dental:Reception Desk', { job_status: 'InProgress' });
  addEntity('Customer', 'Nguyen, David', { first_name: 'David', last_name: 'Nguyen', phone: '(555) 988-1022' });
  addEntity('Customer', 'Oak Street Library', { company_name: 'Oak Street Public Library', terms_id: terms[0]!.id });
  addEntity('Customer', 'Oak Street Library:Reading Tables', { job_status: 'Pending' });
  addEntity('Customer', 'Former Client Co', { is_active: false });
  addEntity('Vendor', 'Hardwood Supply Co', { default_expense_account_id: A('Cost of Goods Sold:Materials'), terms_id: terms[0]!.id, account_number: 'JHW-1180', bill_address: addr('77 Mill St', 'Portland', 'OR', '97201'), type_id: types[2]!.id });
  addEntity('Vendor', 'Rockler Hardware', { default_expense_account_id: A('Cost of Goods Sold:Hardware') });
  addEntity('Vendor', 'Pacific Power', { default_expense_account_id: A('Utilities:Electric'), account_number: '5521-0090' });
  addEntity('Vendor', 'Valley Internet', { default_expense_account_id: A('Utilities:Internet & Phone') });
  addEntity('Vendor', 'City of Juniper Utilities', { default_expense_account_id: A('Utilities:Water & Sewer') });
  addEntity('Vendor', 'Shell', { default_expense_account_id: A('Automobile Expense:Fuel') });
  addEntity('Vendor', 'Harbor Insurance', { default_expense_account_id: A('Insurance Expense:Liability Insurance') });
  addEntity('Vendor', 'Office Depot', { default_expense_account_id: A('Office Supplies') });
  addEntity('Vendor', 'Main Street Properties', { default_expense_account_id: A('Rent Expense'), bill_address: addr('100 Main St Ste 4', 'Juniper', 'OR', '97000') });
  addEntity('Vendor', "Bob's Finishing", { default_expense_account_id: A('Cost of Goods Sold:Subcontractors'), is_1099: true, terms_id: terms[1]!.id, tax_id: '**-***1234' });
  addEntity('Vendor', 'First Valley Bank', { default_expense_account_id: A('Bank Service Charges') });
  addEntity('Vendor', 'Tool Barn', { default_expense_account_id: A('Tools and Small Equipment') });
  addEntity('Vendor', 'State Dept of Revenue');
  addEntity('Employee', 'Lee, Jordan', { first_name: 'Jordan', last_name: 'Lee', hired_date: '2024-03-04' });
  addEntity('Employee', 'Rivera, Sam', { first_name: 'Sam', last_name: 'Rivera', hired_date: '2025-06-16' });
  addEntity('OtherName', 'Owner');
  const E = (name: string) => {
    const e = ent.get(name);
    if (!e) throw new Error(`seed: no name ${name}`);
    return e.id;
  };
  const salesReps: SalesRep[] = [{ id: id(), initials: 'JL', entity_id: E('Lee, Jordan'), is_active: true, edit_seq: 1n }];

  // ---- Items ---------------------------------------------------------------
  const items: Item[] = [];
  const itm = new Map<string, Item>();
  const addItem = (type: ItemType, fullName: string, extra: Partial<Item> = {}): Item => {
    const parts = fullName.split(':');
    const parent = parts.length > 1 ? itm.get(parts.slice(0, -1).join(':')) : undefined;
    const i: Item = { ...blankItem(parts[parts.length - 1]!, type, parent), id: id(), edit_seq: 1n, ...extra };
    items.push(i);
    itm.set(fullName, i);
    return i;
  };
  const twoSided = (sales: string, price: string, cost: string, income: string, expense: string, desc: string): Partial<Item> => ({
    sales_description: desc,
    sales_price: price,
    income_account_id: A(income),
    has_purchase_info: true,
    purchase_description: desc,
    purchase_cost: cost,
    expense_account_id: A(expense),
    sales_tax_code_id: salesTaxCodes[0]!.id,
    ...(sales ? { sku: sales } : {}),
  });
  addItem('Service', 'Design', { sales_description: 'Design and drawings (per hour)', sales_price: '85.00', income_account_id: A('Design Services'), sales_tax_code_id: salesTaxCodes[1]!.id });
  addItem('Service', 'Installation', { sales_description: 'On-site installation (per hour)', sales_price: '65.00', income_account_id: A('Sales:Custom Furniture'), sales_tax_code_id: salesTaxCodes[1]!.id });
  addItem('Service', 'Refinishing', { sales_description: 'Refinishing labor (per hour)', sales_price: '55.00', income_account_id: A('Sales:Repairs & Refinishing'), sales_tax_code_id: salesTaxCodes[1]!.id });
  addItem('Service', 'Finishing Subcontract', twoSided('', '0.00', '0.00', 'Sales:Custom Furniture', 'Cost of Goods Sold:Subcontractors', 'Spray finishing (subcontracted)'));
  addItem('NonInventory', 'Lumber', { sales_description: 'Lumber' });
  addItem('NonInventory', 'Lumber:Walnut', twoSided('LBR-WAL', '18.50', '12.75', 'Sales:Custom Furniture', 'Cost of Goods Sold:Materials', 'Black walnut, 4/4 FAS (per bd ft)'));
  addItem('NonInventory', 'Lumber:White Oak', twoSided('LBR-WOK', '11.25', '7.90', 'Sales:Custom Furniture', 'Cost of Goods Sold:Materials', 'White oak, quartersawn 4/4 (per bd ft)'));
  addItem('NonInventory', 'Lumber:Maple', twoSided('LBR-MAP', '9.40', '6.15', 'Sales:Cabinetry', 'Cost of Goods Sold:Materials', 'Hard maple 4/4 (per bd ft)'));
  addItem('NonInventory', 'Lumber:Baltic Birch Ply', twoSided('PLY-BB34', '96.00', '68.40', 'Sales:Cabinetry', 'Cost of Goods Sold:Materials', 'Baltic birch plywood 3/4" 5x5 sheet'));
  addItem('NonInventory', 'Hardware', { sales_description: 'Hardware' });
  addItem('NonInventory', 'Hardware:Drawer Slides', twoSided('HW-SL21', '24.00', '15.80', 'Sales:Cabinetry', 'Cost of Goods Sold:Hardware', '21" full-extension soft-close slides (pair)'));
  addItem('NonInventory', 'Hardware:Hinges', twoSided('HW-HG35', '9.50', '5.25', 'Sales:Cabinetry', 'Cost of Goods Sold:Hardware', '35mm concealed hinge, soft close'));
  addItem('NonInventory', 'Hardware:Pulls', twoSided('HW-PL', '14.00', '8.10', 'Sales:Cabinetry', 'Cost of Goods Sold:Hardware', 'Brushed brass pull, 5"'));
  addItem('NonInventory', 'Finish Supplies', twoSided('FIN', '0.00', '0.00', 'Sales:Custom Furniture', 'Cost of Goods Sold:Materials', 'Oils, waxes and abrasives'));
  addItem('OtherCharge', 'Delivery', { sales_description: 'Delivery and setup', sales_price: '75.00', income_account_id: A('Delivery Income'), sales_tax_code_id: salesTaxCodes[1]!.id });
  addItem('Discount', 'Repeat Customer Discount', { sales_description: '10% repeat customer discount', sales_price: '10', is_percent: true, income_account_id: A('Sales') });
  addItem('SalesTax', 'Juniper County Sales Tax', { sales_description: 'Juniper County', tax_rate: '7.25', tax_agency_id: E('State Dept of Revenue') });
  addItem('Service', 'Old Service Plan', { is_active: false, sales_price: '40.00', income_account_id: A('Sales') });
  const I = (name: string) => {
    const i = itm.get(name);
    if (!i) throw new Error(`seed: no item ${name}`);
    return i;
  };

  // ---- Transactions --------------------------------------------------------
  const txns: Txn[] = [];
  let checkNo = 1041;
  const start = firstOfMonth(addMonths(today, -4));
  const day = (monthOffset: number, d: number) => {
    const m = addMonths(start, monthOffset);
    const date = addDays(m, d - 1);
    return date > today ? today : date;
  };
  const txn = (type: Txn['txn_type'], date: string, fields: Partial<Txn>, lines: TxnLine[] = []): Txn => {
    const t: Txn = { ...blankTxn(type, date), id: id(), ...fields, lines };
    t.lines = t.lines.map((l) => ({ ...l, id: id() }));
    txns.push(t);
    return t;
  };
  const expense = (account: string, amount: number, extra: Partial<TxnLine> = {}): TxnLine => ({
    ...blankLine('Account'),
    account_id: A(account),
    amount,
    ...extra,
  });
  const itemLine = (name: string, qty: string, side: 'buy' | 'sell', extra: Partial<TxnLine> = {}): TxnLine => {
    const it = I(name);
    const rate = (side === 'buy' ? it.purchase_cost : it.sales_price) ?? '0';
    return {
      ...blankLine('Item'),
      item_id: it.id,
      description: side === 'buy' ? it.purchase_description : it.sales_description,
      quantity: qty,
      rate,
      amount: extend(qty, rate) ?? 0,
      sales_tax_code_id: side === 'sell' ? it.sales_tax_code_id : null,
      ...extra,
    };
  };
  const journal = (account: string, amount: number, extra: Partial<TxnLine> = {}): TxnLine => ({
    ...blankLine('Journal'),
    account_id: A(account),
    amount,
    ...extra,
  });

  // Opening balances.
  txn('JournalEntry', day(0, 1), { number: 'OB-1', memo: 'Opening balances' }, [
    journal('Checking', 2_450_000),
    journal('Savings', 4_000_000),
    journal('Petty Cash', 25_000),
    journal('Shop Equipment:Original Cost', 1_850_000),
    journal('Vehicles', 2_200_000),
    journal('Equipment Loan', -1_200_000),
    journal('Opening Balance Equity', -9_325_000),
  ]);

  for (let m = 0; m <= 4; m++) {
    const check = (d: number, payee: string, lines: TxnLine[], extra: Partial<Txn> = {}) =>
      txn('Check', day(m, d), {
        number: String(checkNo++),
        entity_id: E(payee),
        account_id: A('Checking'),
        amount: lines.reduce((s, l) => s + l.amount, 0),
        ...extra,
      }, lines);
    const charge = (d: number, payee: string, account: string, amount: number, memo: string, card = 'Visa') =>
      txn('CreditCardCharge', day(m, d), { entity_id: E(payee), account_id: A(card), amount, memo }, [expense(account, amount)]);

    check(1, 'Main Street Properties', [expense('Rent Expense', 185_000)], { memo: 'Shop rent' });
    check(6, 'Pacific Power', [expense('Utilities:Electric', between(21_000, 34_000))], { memo: 'Acct 5521-0090' });
    check(9, 'City of Juniper Utilities', [expense('Utilities:Water & Sewer', between(6_000, 9_000))]);
    charge(12, 'Valley Internet', 'Utilities:Internet & Phone', 11_999, 'Business fiber');
    charge(3, 'Shell', 'Automobile Expense:Fuel', between(5_500, 9_800), 'Truck fuel');
    charge(17, 'Shell', 'Automobile Expense:Fuel', between(5_500, 9_800), 'Truck fuel');
    charge(22, 'Office Depot', 'Office Supplies', between(2_000, 12_000), 'Printer paper, toner', 'American Express');
    if (m % 2 === 0) {
      check(15, 'Harbor Insurance', [
        expense('Insurance Expense:Liability Insurance', 64_000),
        expense("Insurance Expense:Workers' Compensation", 41_550),
      ]);
    }
    // Materials for the jobs, partly billable.
    check(8, 'Hardwood Supply Co', [
      itemLine('Lumber:Walnut', String(between(40, 120)), 'buy', { entity_id: E('Acme Corp:Lobby Bench'), billable: 'Billable' }),
      itemLine('Lumber:White Oak', String(between(60, 200)), 'buy', { entity_id: E('Acme Corp:Lobby Bench') }),
      itemLine('Lumber:Baltic Birch Ply', String(between(4, 12)), 'buy', { entity_id: E('Baker, Ellen:Kitchen Cabinets') }),
    ], { memo: 'Invoice HW-' + (2210 + m) });
    check(20, 'Rockler Hardware', [
      itemLine('Hardware:Drawer Slides', String(between(6, 18)), 'buy', { entity_id: E('Baker, Ellen:Kitchen Cabinets') }),
      itemLine('Hardware:Hinges', String(between(10, 30)), 'buy'),
    ]);
    if (m === 2) check(24, 'Tool Barn', [expense('Tools and Small Equipment', 48_995, { description: 'Track saw' })]);
    // Subcontractor bills, paid the next month.
    const bill = txn('Bill', day(m, 11), {
      number: `BF-${300 + m}`,
      entity_id: E("Bob's Finishing"),
      terms_id: terms[1]!.id,
      due_date: addDays(day(m, 11), 15),
      amount: 0,
    }, [expense('Cost of Goods Sold:Subcontractors', between(80_000, 160_000), { entity_id: E('Acme Corp:Lobby Bench'), description: 'Spray finish' })]);
    bill.amount = bill.lines.reduce((s, l) => s + l.amount, 0);
    if (m < 4) {
      txn('BillPaymentCheck', day(m + 1, 2) < today ? day(m + 1, 2) : today, {
        number: String(checkNo++),
        entity_id: E("Bob's Finishing"),
        account_id: A('Checking'),
        amount: bill.amount,
        applications: [{ target_txn_id: bill.id, amount: bill.amount, discount: 0, discount_account_id: null, discount_class_id: null, credit_txn_id: null, credit_amount: 0 }],
      });
    }
    // Sales: an invoice or two a month, paid into Undeposited Funds, then deposited.
    const customers = ['Acme Corp:Lobby Bench', 'Baker, Ellen:Kitchen Cabinets', 'Thompson Dental:Reception Desk', 'Riverside Cafe:Patio Furniture', 'Nguyen, David'];
    for (let k = 0; k < 2; k++) {
      const cust = customers[(m * 2 + k) % customers.length]!;
      const inv = txn('Invoice', day(m, 5 + k * 10), {
        number: String(2100 + m * 2 + k),
        entity_id: E(cust),
        terms_id: terms[0]!.id,
        due_date: addDays(day(m, 5 + k * 10), 30),
        sales_tax_item_id: I('Juniper County Sales Tax').id,
        sales_rep_id: salesReps[0]!.id,
        memo: 'Thank you for your business.',
      }, [
        itemLine('Design', String(between(3, 12)), 'sell'),
        itemLine(k ? 'Lumber:Maple' : 'Lumber:Walnut', String(between(30, 90)), 'sell'),
        itemLine('Installation', String(between(4, 16)), 'sell'),
        itemLine('Delivery', '1', 'sell'),
      ]);
      // Older invoices are paid; the newest stay open (so A/R has something in it).
      if (m < 3 || (m === 3 && k === 0)) {
        const payDate = addDays(inv.date, 18);
        if (payDate <= today) {
          txn('ReceivePayment', payDate, {
            entity_id: inv.entity_id,
            number: String(5000 + between(1, 999)),
            payment_method_id: paymentMethods[1]!.id,
            amount: -1, // filled after posting (invoice total includes tax)
            applications: [{ target_txn_id: inv.id, amount: -1, discount: 0, discount_account_id: null, discount_class_id: null, credit_txn_id: null, credit_amount: 0 }],
          });
        }
      }
    }
    // Monthly deposit of what's waiting in Undeposited Funds (filled after posting).
    if (day(m, 28) < today) {
      txn('Deposit', day(m, 28), { account_id: A('Checking'), memo: 'Deposit', extra: { fixture_sweep_undeposited: true } });
    }
    txn('Check', day(m, 28), {
      account_id: A('Checking'),
      number: null,
      entity_id: E('First Valley Bank'),
      memo: 'Service charge',
      amount: 1_500,
    }, [expense('Bank Service Charges', 1_500)]);
    txn('Deposit', day(m, 28), { account_id: A('Savings'), memo: 'Interest' }, [
      { ...blankLine('DepositLine'), account_id: A('Interest Income'), amount: between(800, 1_400) },
    ]);
    if (m === 1 || m === 3) {
      txn('Transfer', day(m, 14), { account_id: A('Savings'), to_account_id: A('Checking'), amount: 500_000, memo: 'Funds transfer' });
    }
    // Pay the Visa off from checking each month (a check to the card account).
    if (m > 0) {
      check(25, 'First Valley Bank', [expense('Visa', between(40_000, 60_000), { description: 'Visa payment' })], { memo: 'Visa payment' });
    }
    // Loan payment: principal and interest.
    check(27, 'First Valley Bank', [expense('Equipment Loan', 35_000), expense('Interest Expense', between(5_000, 6_000))], { memo: 'Equipment loan payment' });
  }
  txn('Check', day(2, 18), { account_id: A('Checking'), number: String(checkNo++), entity_id: E('Owner'), amount: 300_000, memo: 'Owner draw' }, [
    expense("Owner's Equity:Owner Draws", 300_000),
  ]);
  // Something for the estimate list (non-posting).
  txn('Estimate', today, { number: 'E-118', entity_id: E('Oak Street Library:Reading Tables') }, [itemLine('Design', '10', 'sell'), itemLine('Lumber:White Oak', '240', 'sell')]);

  const info: CompanyInfo = {
    name: 'Juniper Hill Woodworks',
    legal_name: 'Juniper Hill Woodworks LLC',
    address: addr('512 Foundry Ave', 'Juniper', 'OR', '97000'),
    legal_address: addr('512 Foundry Ave', 'Juniper', 'OR', '97000'),
    phone: '(555) 200-1919',
    email: 'books@juniperhill.example',
    website: null,
    ein: '00-0000000',
    fiscal_year_start_month: 1,
    tax_year_start_month: 1,
    tax_form: 'Form1065',
    home_currency: 'USD',
    preferences: {
      use_account_numbers: false,
      use_class_tracking: true,
      prompt_for_class: false,
      warn_days_past: 90,
      warn_days_future: 30,
      auto_number_journal_entries: true,
      inventory_enabled: false,
      estimates_enabled: true,
      sales_orders_enabled: false,
      progress_invoicing: false,
      sales_tax_enabled: true,
      sales_tax_on_cash_basis: false,
      reports_cash_basis: false,
      billable_by_default: true,
      default_markup_percent: null,
      use_undeposited_funds: true,
      auto_apply_payments: true,
      default_check_account: A('Checking'),
      default_deposit_account: A('Checking'),
      multi_currency: false,
    },
  };

  return { info, accounts, entities, items, classes, terms, paymentMethods, salesTaxCodes, salesReps, types, txns };
}

/**
 * Many names and accounts, for feeling QuickFill and the lists at scale
 * (`?fixtures=stress`).
 */
export function stressNames(count: number): Entity[] {
  const id = (n: number) => `00000000-0000-4000-9000-${n.toString(16).padStart(12, '0')}`;
  const first = ['Alder', 'Birch', 'Cedar', 'Douglas', 'Elm', 'Fir', 'Gum', 'Hemlock', 'Ironwood', 'Juniper', 'Koa', 'Larch'];
  const second = ['Supply', 'Builders', 'Logistics', 'Freight', 'Tooling', 'Partners', 'Holdings', 'Services', 'Works', 'Traders'];
  return Array.from({ length: count }, (_, i) => ({
    ...blankEntity(`${first[i % first.length]} ${second[Math.floor(i / first.length) % second.length]} ${i + 1}`, 'Vendor'),
    id: id(i + 1),
    edit_seq: 1n,
  }));
}
