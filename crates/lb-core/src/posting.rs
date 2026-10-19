//! Posting rules: from a transaction as entered to its double-entry postings.
//!
//! Each transaction type has one rule, written out below in the same terms an
//! accountant would use ("credit the bank, debit each expense line"). The rules
//! are pure: everything they need from the rest of the books comes through
//! [`PostingContext`], so they are tested without a database and the store can call
//! them inside its write transaction.
//!
//! Invariants every rule's result satisfies, checked once at the end:
//! - postings sum to zero;
//! - every posting's account exists, is not non-posting, and lines in A/R or A/P
//!   carry a name.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::account::{AccountRole, AccountType};
use crate::entity::EntityKind;
use crate::ids::{AccountId, EntityId, ItemId, SalesTaxCodeId, TxnId};
use crate::item::{Item, ItemType};
use crate::money::Money;
use crate::txn::{HeaderEntity, LineKind, Posting, Txn, TxnLine, TxnType};

/// What an account looks like to the posting rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountInfo {
    pub account_type: AccountType,
    pub is_active: bool,
}

/// What an open transaction being paid looks like to a payment's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenTxnInfo {
    pub txn_type: TxnType,
    pub entity_id: Option<EntityId>,
    /// The A/R or A/P account it sits in.
    pub ar_ap_account_id: Option<AccountId>,
}

/// The rest of the books, as far as posting needs them.
pub trait PostingContext {
    fn account(&self, id: AccountId) -> Option<AccountInfo>;
    /// The account playing a role. `None` if the company has none yet; the rule then
    /// fails with [`PostingError::MissingRoleAccount`] and the store creates one and
    /// tries again.
    fn role_account(&self, role: AccountRole) -> Option<AccountId>;
    fn item(&self, id: ItemId) -> Option<Item>;
    fn entity_kind(&self, id: EntityId) -> Option<EntityKind>;
    /// Is a sales tax code taxable. Unknown codes are not.
    fn tax_code_is_taxable(&self, id: SalesTaxCodeId) -> bool;
    /// An open transaction a payment applies to.
    fn open_txn(&self, id: TxnId) -> Option<OpenTxnInfo>;
    /// Average cost per unit of an inventory item as of `date`, not counting
    /// `excluding` (the transaction being posted, when it is an edit).
    fn average_cost(&self, item: ItemId, date: jiff::civil::Date, excluding: TxnId) -> Decimal;
}

/// Why a transaction cannot be posted. Messages are for people.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "code", content = "detail")]
pub enum PostingError {
    #[error("the transaction does not balance: debits and credits differ by {0}")]
    Unbalanced(Money),
    #[error("choose an account")]
    MissingAccount,
    #[error("choose the account to transfer to")]
    MissingToAccount,
    #[error("the account doesn't exist")]
    UnknownAccount(AccountId),
    #[error("{0} is inactive")]
    InactiveAccount(AccountId),
    #[error("a non-posting account can't be used here")]
    NonPostingAccount(AccountId),
    #[error("this needs a {expected} account")]
    WrongAccountType {
        account: AccountId,
        expected: String,
    },
    #[error("the company has no {0:?} account yet")]
    MissingRoleAccount(AccountRole),
    #[error("the item doesn't exist")]
    UnknownItem(ItemId),
    #[error("{0} has no account to post to")]
    ItemWithoutAccount(String),
    #[error("choose a name")]
    MissingEntity,
    #[error("the name doesn't exist")]
    UnknownEntity(EntityId),
    #[error("this needs a {expected}")]
    WrongEntityKind { entity: EntityId, expected: String },
    #[error("lines in Accounts Receivable or Accounts Payable need a customer or vendor")]
    ArApLineWithoutName,
    #[error("a transfer's accounts must differ")]
    TransferToSameAccount,
    #[error("the amount must be more than zero")]
    NonPositiveAmount,
    #[error("the transaction has no lines")]
    NoLines,
    #[error("payments applied ({applied}) exceed the payment ({amount})")]
    OverApplied { applied: Money, amount: Money },
    #[error("the transaction being paid doesn't exist or isn't open")]
    UnknownTarget(TxnId),
    #[error("a payment can only pay this customer's or vendor's transactions")]
    TargetForOtherName(TxnId),
    #[error("the transaction being paid is in a different A/R or A/P account")]
    TargetInOtherAccount(TxnId),
    #[error("a discount needs an account")]
    DiscountWithoutAccount,
    #[error("{0} lines aren't allowed on this form")]
    LineKindNotAllowed(LineKind),
    #[error("{0} items aren't supported on this form yet")]
    UnsupportedItem(String),
    #[error("an amount is too large")]
    Overflow,
}

impl std::fmt::Display for LineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LineKind::Account => "Expense",
            LineKind::Item => "Item",
            LineKind::Journal => "Journal",
            LineKind::DepositLine => "Deposit",
            LineKind::CashBack => "Cash back",
            LineKind::Text => "Text",
        };
        f.write_str(s)
    }
}

/// What posting a transaction produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Posted {
    pub postings: Vec<Posting>,
    /// The form's total, as it shows at the bottom: a check's amount, an invoice's
    /// total including tax, a payment's amount.
    pub total: Money,
    /// Sales tax computed on a sales form.
    pub sales_tax: Money,
}

/// Post a transaction. Pending and non-posting transactions post nothing. Voided
/// transactions keep their postings' shape with every amount zero, so they still
/// appear (as VOID) in registers.
pub fn post(txn: &Txn, ctx: &dyn PostingContext) -> Result<Posted, PostingError> {
    if !txn.txn_type.is_posting() {
        let total = txn.lines_total();
        return Ok(Posted {
            postings: Vec::new(),
            total,
            sales_tax: Money::ZERO,
        });
    }
    check_header_entity(txn, ctx)?;
    let mut b = Builder::new(txn, ctx);
    match txn.txn_type {
        TxnType::Check | TxnType::Paycheck | TxnType::LiabilityCheck => {
            b.cash_out(&[
                AccountType::Bank,
                AccountType::OtherCurrentAsset,
                AccountType::CreditCard,
            ])?;
        }
        TxnType::CreditCardCharge => b.cash_out(&[AccountType::CreditCard])?,
        TxnType::CreditCardCredit => b.cash_in_refund(&[AccountType::CreditCard])?,
        TxnType::Deposit => b.deposit()?,
        TxnType::Transfer => b.transfer()?,
        TxnType::JournalEntry => b.journal()?,
        TxnType::Bill | TxnType::ItemReceipt => b.payable(Direction::Normal)?,
        TxnType::VendorCredit => b.payable(Direction::Reversed)?,
        TxnType::Invoice | TxnType::StatementCharge => {
            b.sale(SaleSide::Receivable, Direction::Normal)?;
        }
        TxnType::CreditMemo => b.sale(SaleSide::Receivable, Direction::Reversed)?,
        TxnType::SalesReceipt => b.sale(SaleSide::Deposit, Direction::Normal)?,
        TxnType::ReceivePayment => b.receive_payment()?,
        TxnType::BillPaymentCheck => {
            b.bill_payment(&[AccountType::Bank, AccountType::OtherCurrentAsset])?;
        }
        TxnType::BillPaymentCreditCard => b.bill_payment(&[AccountType::CreditCard])?,
        TxnType::SalesTaxPaymentCheck => b.sales_tax_payment()?,
        TxnType::InventoryAdjustment => b.inventory_adjustment()?,
        TxnType::BuildAssembly => b.build_assembly()?,
        TxnType::PurchaseOrder
        | TxnType::Estimate
        | TxnType::SalesOrder
        | TxnType::TimeTracking => {
            unreachable!("non-posting types return above")
        }
    }
    let mut posted = b.finish()?;
    if txn.is_pending {
        posted.postings.clear();
    } else if txn.is_void {
        for p in &mut posted.postings {
            p.amount = Money::ZERO;
            p.quantity = p.quantity.map(|_| Decimal::ZERO);
        }
        posted.total = Money::ZERO;
        posted.sales_tax = Money::ZERO;
    }
    Ok(posted)
}

fn check_header_entity(txn: &Txn, ctx: &dyn PostingContext) -> Result<(), PostingError> {
    let expected = match txn.txn_type.header_entity() {
        HeaderEntity::None | HeaderEntity::Any => {
            if let Some(e) = txn.entity_id {
                ctx.entity_kind(e).ok_or(PostingError::UnknownEntity(e))?;
            }
            return Ok(());
        }
        HeaderEntity::Customer => EntityKind::Customer,
        HeaderEntity::Vendor => EntityKind::Vendor,
        HeaderEntity::Employee => EntityKind::Employee,
    };
    let e = txn.entity_id.ok_or(PostingError::MissingEntity)?;
    let kind = ctx.entity_kind(e).ok_or(PostingError::UnknownEntity(e))?;
    // QBD lets a liability check or sales tax payment go to any name that is a
    // vendor; paychecks only to employees. Customers only on sales forms.
    if kind != expected {
        return Err(PostingError::WrongEntityKind {
            entity: e,
            expected: expected.as_str().to_owned(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Normal,
    Reversed,
}

impl Direction {
    fn apply(self, m: Money) -> Money {
        match self {
            Direction::Normal => m,
            Direction::Reversed => -m,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SaleSide {
    Receivable,
    Deposit,
}

struct Builder<'a> {
    txn: &'a Txn,
    ctx: &'a dyn PostingContext,
    postings: Vec<Posting>,
    total: Money,
    sales_tax: Money,
}

impl<'a> Builder<'a> {
    fn new(txn: &'a Txn, ctx: &'a dyn PostingContext) -> Self {
        Builder {
            txn,
            ctx,
            postings: Vec::new(),
            total: Money::ZERO,
            sales_tax: Money::ZERO,
        }
    }

    fn finish(self) -> Result<Posted, PostingError> {
        let sum: Money = self.postings.iter().map(|p| p.amount).sum();
        if !sum.is_zero() {
            return Err(PostingError::Unbalanced(sum));
        }
        for p in &self.postings {
            let info = self
                .ctx
                .account(p.account_id)
                .ok_or(PostingError::UnknownAccount(p.account_id))?;
            if info.account_type == AccountType::NonPosting {
                return Err(PostingError::NonPostingAccount(p.account_id));
            }
            if info.account_type.requires_name() && p.entity_id.is_none() && !p.amount.is_zero() {
                return Err(PostingError::ArApLineWithoutName);
            }
        }
        Ok(Posted {
            postings: merge_zero_lines(self.postings),
            total: self.total,
            sales_tax: self.sales_tax,
        })
    }

    fn account_of_type(
        &self,
        id: Option<AccountId>,
        types: &[AccountType],
    ) -> Result<AccountId, PostingError> {
        let id = id.ok_or(PostingError::MissingAccount)?;
        let info = self
            .ctx
            .account(id)
            .ok_or(PostingError::UnknownAccount(id))?;
        if !types.is_empty() && !types.contains(&info.account_type) {
            return Err(PostingError::WrongAccountType {
                account: id,
                expected: types
                    .iter()
                    .map(|t| t.label())
                    .collect::<Vec<_>>()
                    .join(" or "),
            });
        }
        Ok(id)
    }

    fn role(&self, role: AccountRole) -> Result<AccountId, PostingError> {
        self.ctx
            .role_account(role)
            .ok_or(PostingError::MissingRoleAccount(role))
    }

    fn ar_ap(&self, role: AccountRole) -> Result<AccountId, PostingError> {
        let wanted = role.default_shape().0;
        match self.txn.ar_ap_account_id {
            Some(id) => self.account_of_type(Some(id), &[wanted]),
            None => self.role(role),
        }
    }

    fn header(&mut self, account: AccountId, amount: Money) {
        let mut p = Posting::new(account, amount);
        p.entity_id = self.txn.entity_id;
        p.class_id = self.txn.class_id;
        p.memo.clone_from(&self.txn.memo);
        self.postings.push(p);
    }

    fn line_posting(&mut self, line: &TxnLine, account: AccountId, amount: Money) -> &mut Posting {
        let mut p = Posting::new(account, amount);
        p.entity_id = line.entity_id;
        p.class_id = line.class_id.or(self.txn.class_id);
        p.item_id = line.item_id;
        p.line_id = Some(line.id);
        p.memo = line.description.clone().or_else(|| self.txn.memo.clone());
        self.postings.push(p);
        self.postings.last_mut().expect("just pushed")
    }

    fn item(&self, id: ItemId) -> Result<Item, PostingError> {
        self.ctx.item(id).ok_or(PostingError::UnknownItem(id))
    }

    /// Debit (in `dir`) one purchase-side line: an expense line's account, or an
    /// item's purchase account (inventory goes to its asset account with a
    /// quantity).
    fn purchase_line(&mut self, line: &TxnLine, dir: Direction) -> Result<Money, PostingError> {
        match line.kind {
            LineKind::Account => {
                let acct = self.account_of_type(line.account_id, &[])?;
                self.line_posting(line, acct, dir.apply(line.amount));
                Ok(line.amount)
            }
            LineKind::Item => {
                let item_id = line
                    .item_id
                    .ok_or(PostingError::UnknownItem(ItemId::default()))?;
                let item = self.item(item_id)?;
                match item.item_type {
                    ItemType::Subtotal | ItemType::Group => Ok(Money::ZERO),
                    ItemType::Inventory | ItemType::InventoryAssembly => {
                        let acct = item
                            .asset_account_id
                            .map_or_else(|| self.role(AccountRole::InventoryAsset), Ok)?;
                        let qty = line.quantity.unwrap_or(Decimal::ZERO);
                        let p = self.line_posting(line, acct, dir.apply(line.amount));
                        p.quantity = Some(match dir {
                            Direction::Normal => qty,
                            Direction::Reversed => -qty,
                        });
                        Ok(line.amount)
                    }
                    ItemType::Payment | ItemType::SalesTaxGroup => Err(
                        PostingError::UnsupportedItem(item.item_type.label().to_owned()),
                    ),
                    ItemType::SalesTax => {
                        let acct = self.role(AccountRole::SalesTaxPayable)?;
                        let agency = item.tax_agency_id;
                        let p = self.line_posting(line, acct, dir.apply(line.amount));
                        p.entity_id = p.entity_id.or(agency);
                        Ok(line.amount)
                    }
                    _ => {
                        let acct = purchase_account(&item).ok_or_else(|| {
                            PostingError::ItemWithoutAccount(item.full_name.clone())
                        })?;
                        self.line_posting(line, acct, dir.apply(line.amount));
                        Ok(line.amount)
                    }
                }
            }
            LineKind::Text => Ok(Money::ZERO),
            other => Err(PostingError::LineKindNotAllowed(other)),
        }
    }

    fn purchase_lines(&mut self, dir: Direction) -> Result<Money, PostingError> {
        let mut total = Money::ZERO;
        for line in &self.txn.lines {
            let amt = self.purchase_line(line, dir)?;
            total = total.checked_add(amt).ok_or(PostingError::Overflow)?;
        }
        Ok(total)
    }

    /// Check, paycheck, liability check, credit card charge: credit the account
    /// drawn on, debit each line.
    fn cash_out(&mut self, types: &[AccountType]) -> Result<(), PostingError> {
        let acct = self.account_of_type(self.txn.account_id, types)?;
        let total = self.purchase_lines(Direction::Normal)?;
        self.header(acct, -total);
        self.total = total;
        Ok(())
    }

    /// Credit card credit (a refund on the card): the reverse of a charge.
    fn cash_in_refund(&mut self, types: &[AccountType]) -> Result<(), PostingError> {
        let acct = self.account_of_type(self.txn.account_id, types)?;
        let total = self.purchase_lines(Direction::Reversed)?;
        self.header(acct, total);
        self.total = total;
        Ok(())
    }

    /// Bill, item receipt (normal) and vendor credit (reversed): A/P against the lines.
    fn payable(&mut self, dir: Direction) -> Result<(), PostingError> {
        let ap = self.ar_ap(AccountRole::AccountsPayable)?;
        let total = self.purchase_lines(dir)?;
        self.header(ap, dir.apply(-total));
        self.total = total;
        Ok(())
    }

    /// Deposit: debit the bank with the net deposit, credit each line's source,
    /// debit the cash-back account.
    fn deposit(&mut self) -> Result<(), PostingError> {
        let bank = self.account_of_type(
            self.txn.account_id,
            &[
                AccountType::Bank,
                AccountType::OtherCurrentAsset,
                AccountType::CreditCard,
            ],
        )?;
        let mut gross = Money::ZERO;
        let mut cash_back = Money::ZERO;
        for line in &self.txn.lines {
            match line.kind {
                LineKind::DepositLine | LineKind::Account => {
                    let acct = self.account_of_type(line.account_id, &[])?;
                    self.line_posting(line, acct, -line.amount);
                    gross = gross
                        .checked_add(line.amount)
                        .ok_or(PostingError::Overflow)?;
                }
                LineKind::CashBack => {
                    let acct = self.account_of_type(line.account_id, &[])?;
                    self.line_posting(line, acct, line.amount);
                    cash_back = cash_back
                        .checked_add(line.amount)
                        .ok_or(PostingError::Overflow)?;
                }
                LineKind::Text => {}
                other => return Err(PostingError::LineKindNotAllowed(other)),
            }
        }
        let net = gross - cash_back;
        let mut p = Posting::new(bank, net);
        p.class_id = self.txn.class_id;
        p.memo.clone_from(&self.txn.memo);
        self.postings.push(p);
        self.total = net;
        Ok(())
    }

    fn transfer(&mut self) -> Result<(), PostingError> {
        let from = self.account_of_type(self.txn.account_id, &[])?;
        let to = self
            .txn
            .to_account_id
            .ok_or(PostingError::MissingToAccount)?;
        let to = self.account_of_type(Some(to), &[])?;
        if from == to {
            return Err(PostingError::TransferToSameAccount);
        }
        if !self.txn.amount.is_positive() {
            return Err(PostingError::NonPositiveAmount);
        }
        let amount = self.txn.amount;
        let mut out = Posting::new(from, -amount);
        out.memo.clone_from(&self.txn.memo);
        out.class_id = self.txn.class_id;
        let mut inn = Posting::new(to, amount);
        inn.memo.clone_from(&self.txn.memo);
        inn.class_id = self.txn.class_id;
        self.postings.push(out);
        self.postings.push(inn);
        self.total = amount;
        Ok(())
    }

    fn journal(&mut self) -> Result<(), PostingError> {
        if self.txn.lines.iter().all(|l| l.kind == LineKind::Text) {
            return Err(PostingError::NoLines);
        }
        let mut debits = Money::ZERO;
        for line in &self.txn.lines {
            match line.kind {
                LineKind::Journal => {
                    let acct = self.account_of_type(line.account_id, &[])?;
                    self.line_posting(line, acct, line.amount);
                    if line.amount.is_positive() {
                        debits += line.amount;
                    }
                }
                LineKind::Text => {}
                other => return Err(PostingError::LineKindNotAllowed(other)),
            }
        }
        self.total = debits;
        Ok(())
    }

    /// Invoice, statement charge, sales receipt (normal) and credit memo
    /// (reversed): credit income per line, credit sales tax, debit A/R (or the
    /// deposit account), and move inventory cost to COGS.
    fn sale(&mut self, side: SaleSide, dir: Direction) -> Result<(), PostingError> {
        let customer_taxable = self
            .txn
            .customer_tax_code_id
            .is_none_or(|c| self.ctx.tax_code_is_taxable(c));
        let mut subtotal = Money::ZERO;
        let mut taxable_base = Money::ZERO;
        for line in &self.txn.lines {
            match line.kind {
                LineKind::Text => continue,
                LineKind::Account => {
                    let acct = self.account_of_type(line.account_id, &[])?;
                    self.line_posting(line, acct, dir.apply(-line.amount));
                    subtotal = subtotal
                        .checked_add(line.amount)
                        .ok_or(PostingError::Overflow)?;
                    if line
                        .sales_tax_code_id
                        .is_some_and(|c| self.ctx.tax_code_is_taxable(c))
                    {
                        taxable_base += line.amount;
                    }
                }
                LineKind::Item => {
                    let item_id = line
                        .item_id
                        .ok_or(PostingError::UnknownItem(ItemId::default()))?;
                    let item = self.item(item_id)?;
                    match item.item_type {
                        ItemType::Subtotal | ItemType::Group => continue,
                        ItemType::Payment | ItemType::SalesTaxGroup | ItemType::SalesTax => {
                            return Err(PostingError::UnsupportedItem(
                                item.item_type.label().to_owned(),
                            ));
                        }
                        _ => {}
                    }
                    let income = item
                        .income_account_id
                        .ok_or_else(|| PostingError::ItemWithoutAccount(item.full_name.clone()))?;
                    self.line_posting(line, income, dir.apply(-line.amount));
                    subtotal = subtotal
                        .checked_add(line.amount)
                        .ok_or(PostingError::Overflow)?;
                    let line_taxable = line
                        .sales_tax_code_id
                        .or(item.sales_tax_code_id)
                        .is_some_and(|c| self.ctx.tax_code_is_taxable(c));
                    if line_taxable {
                        taxable_base += line.amount;
                    }
                    if item.item_type.is_inventory() {
                        let qty = line.quantity.unwrap_or(Decimal::ZERO);
                        if !qty.is_zero() {
                            let unit = self.ctx.average_cost(item.id, self.txn.date, self.txn.id);
                            let cost = Money::extend(qty, unit).ok_or(PostingError::Overflow)?;
                            let cogs = item
                                .expense_account_id
                                .map_or_else(|| self.role(AccountRole::CostOfGoodsSold), Ok)?;
                            let asset = item
                                .asset_account_id
                                .map_or_else(|| self.role(AccountRole::InventoryAsset), Ok)?;
                            let mut c = Posting::new(cogs, dir.apply(cost));
                            c.item_id = Some(item.id);
                            c.line_id = Some(line.id);
                            c.entity_id = line.entity_id.or(self.txn.entity_id);
                            c.class_id = line.class_id.or(self.txn.class_id);
                            self.postings.push(c);
                            let mut a = Posting::new(asset, dir.apply(-cost));
                            a.item_id = Some(item.id);
                            a.line_id = Some(line.id);
                            a.quantity = Some(dir.apply_qty(-qty));
                            a.class_id = line.class_id.or(self.txn.class_id);
                            self.postings.push(a);
                        }
                    }
                }
                other => return Err(PostingError::LineKindNotAllowed(other)),
            }
        }

        // Sales tax: one posting per agency, on the taxable lines.
        let mut tax = Money::ZERO;
        if customer_taxable
            && !taxable_base.is_zero()
            && let Some(tax_item_id) = self.txn.sales_tax_item_id
        {
            for (agency, rate) in self.tax_components(tax_item_id)? {
                let amt = taxable_base.percent(rate).ok_or(PostingError::Overflow)?;
                if amt.is_zero() {
                    continue;
                }
                let acct = self.role(AccountRole::SalesTaxPayable)?;
                let mut p = Posting::new(acct, dir.apply(-amt));
                p.entity_id = agency;
                p.item_id = Some(tax_item_id);
                self.postings.push(p);
                tax += amt;
            }
        }

        let total = subtotal.checked_add(tax).ok_or(PostingError::Overflow)?;
        let debit_account = match side {
            SaleSide::Receivable => self.ar_ap(AccountRole::AccountsReceivable)?,
            SaleSide::Deposit => match self.txn.account_id {
                Some(id) => self.account_of_type(
                    Some(id),
                    &[
                        AccountType::Bank,
                        AccountType::OtherCurrentAsset,
                        AccountType::CreditCard,
                    ],
                )?,
                None => self.role(AccountRole::UndepositedFunds)?,
            },
        };
        self.header(debit_account, dir.apply(total));
        self.total = total;
        self.sales_tax = tax;
        Ok(())
    }

    /// The (agency, rate) pairs a sales tax item or group charges.
    fn tax_components(&self, id: ItemId) -> Result<Vec<(Option<EntityId>, Decimal)>, PostingError> {
        let item = self.item(id)?;
        match item.item_type {
            ItemType::SalesTax => Ok(vec![(
                item.tax_agency_id,
                item.tax_rate.unwrap_or_default(),
            )]),
            ItemType::SalesTaxGroup => {
                let mut out = Vec::new();
                for c in &item.components {
                    let member = self.item(c.item_id)?;
                    out.push((member.tax_agency_id, member.tax_rate.unwrap_or_default()));
                }
                Ok(out)
            }
            other => Err(PostingError::UnsupportedItem(other.label().to_owned())),
        }
    }

    /// The checks every payment's applications share; returns cash applied.
    fn check_applications(&self, ar_ap: AccountId) -> Result<Money, PostingError> {
        let mut applied = Money::ZERO;
        for a in &self.txn.applications {
            let target = self
                .ctx
                .open_txn(a.target_txn_id)
                .ok_or(PostingError::UnknownTarget(a.target_txn_id))?;
            // Which names may pay which (a customer's payment may pay its jobs'
            // invoices) is the store's to check, with the name hierarchy at hand.
            // Here only the A/R or A/P account must agree.
            if let Some(acct) = target.ar_ap_account_id
                && acct != ar_ap
            {
                return Err(PostingError::TargetInOtherAccount(a.target_txn_id));
            }
            if !a.discount.is_zero() && a.discount_account_id.is_none() {
                return Err(PostingError::DiscountWithoutAccount);
            }
            applied = applied
                .checked_add(a.amount)
                .ok_or(PostingError::Overflow)?;
        }
        Ok(applied)
    }

    /// Customer payment: debit the deposit account, credit A/R with the cash; each
    /// discount debits its account and credits A/R. Credits applied move nothing
    /// (the credit memo already credited A/R). Unapplied cash stays as a credit in A/R.
    fn receive_payment(&mut self) -> Result<(), PostingError> {
        let ar = self.ar_ap(AccountRole::AccountsReceivable)?;
        let amount = self.txn.amount;
        if amount.is_negative() {
            return Err(PostingError::NonPositiveAmount);
        }
        let applied = self.check_applications(ar)?;
        if applied > amount {
            return Err(PostingError::OverApplied { applied, amount });
        }
        if !amount.is_zero() {
            let deposit = match self.txn.account_id {
                Some(id) => self.account_of_type(
                    Some(id),
                    &[
                        AccountType::Bank,
                        AccountType::OtherCurrentAsset,
                        AccountType::CreditCard,
                    ],
                )?,
                None => self.role(AccountRole::UndepositedFunds)?,
            };
            self.header(deposit, amount);
            self.header(ar, -amount);
        }
        for a in &self.txn.applications {
            if a.discount.is_zero() {
                continue;
            }
            let acct = a
                .discount_account_id
                .ok_or(PostingError::DiscountWithoutAccount)?;
            let acct = self.account_of_type(Some(acct), &[])?;
            let mut d = Posting::new(acct, a.discount);
            d.class_id = a.discount_class_id.or(self.txn.class_id);
            d.entity_id = self.txn.entity_id;
            self.postings.push(d);
            let mut r = Posting::new(ar, -a.discount);
            r.entity_id = self.txn.entity_id;
            self.postings.push(r);
        }
        self.total = amount;
        Ok(())
    }

    /// Bill payment: credit the bank or card, debit A/P; discounts credit their
    /// account and debit A/P.
    fn bill_payment(&mut self, types: &[AccountType]) -> Result<(), PostingError> {
        let ap = self.ar_ap(AccountRole::AccountsPayable)?;
        let bank = self.account_of_type(self.txn.account_id, types)?;
        let amount = self.txn.amount;
        if amount.is_negative() {
            return Err(PostingError::NonPositiveAmount);
        }
        let applied = self.check_applications(ap)?;
        if applied > amount {
            return Err(PostingError::OverApplied { applied, amount });
        }
        if !amount.is_zero() {
            self.header(bank, -amount);
            self.header(ap, amount);
        }
        for a in &self.txn.applications {
            if a.discount.is_zero() {
                continue;
            }
            let acct = a
                .discount_account_id
                .ok_or(PostingError::DiscountWithoutAccount)?;
            let acct = self.account_of_type(Some(acct), &[])?;
            let mut d = Posting::new(acct, -a.discount);
            d.class_id = a.discount_class_id.or(self.txn.class_id);
            d.entity_id = self.txn.entity_id;
            self.postings.push(d);
            let mut r = Posting::new(ap, a.discount);
            r.entity_id = self.txn.entity_id;
            self.postings.push(r);
        }
        self.total = amount;
        Ok(())
    }

    /// Sales tax payment: credit the bank, debit Sales Tax Payable per line (the
    /// line's account if it names one, else the role account), naming the agency.
    fn sales_tax_payment(&mut self) -> Result<(), PostingError> {
        let bank = self.account_of_type(
            self.txn.account_id,
            &[
                AccountType::Bank,
                AccountType::OtherCurrentAsset,
                AccountType::CreditCard,
            ],
        )?;
        let mut total = Money::ZERO;
        for line in &self.txn.lines {
            match line.kind {
                LineKind::Text => {}
                LineKind::Item | LineKind::Account => {
                    let acct = match line.account_id {
                        Some(a) => self.account_of_type(Some(a), &[])?,
                        None => self.role(AccountRole::SalesTaxPayable)?,
                    };
                    let agency = self.txn.entity_id;
                    let p = self.line_posting(line, acct, line.amount);
                    p.entity_id = p.entity_id.or(agency);
                    total = total
                        .checked_add(line.amount)
                        .ok_or(PostingError::Overflow)?;
                }
                other => return Err(PostingError::LineKindNotAllowed(other)),
            }
        }
        if self.txn.lines.is_empty() {
            // A bare payment of an amount.
            let acct = self.role(AccountRole::SalesTaxPayable)?;
            let mut p = Posting::new(acct, self.txn.amount);
            p.entity_id = self.txn.entity_id;
            self.postings.push(p);
            total = self.txn.amount;
        }
        self.header(bank, -total);
        self.total = total;
        Ok(())
    }

    /// Inventory adjustment: each line moves value (and quantity) in the item's
    /// asset account against the adjustment account in the header.
    fn inventory_adjustment(&mut self) -> Result<(), PostingError> {
        let adj = self.account_of_type(self.txn.account_id, &[])?;
        let mut total = Money::ZERO;
        for line in &self.txn.lines {
            if line.kind == LineKind::Text {
                continue;
            }
            if line.kind != LineKind::Item {
                return Err(PostingError::LineKindNotAllowed(line.kind));
            }
            let item_id = line
                .item_id
                .ok_or(PostingError::UnknownItem(ItemId::default()))?;
            let item = self.item(item_id)?;
            if !item.item_type.is_inventory() {
                return Err(PostingError::UnsupportedItem(
                    item.item_type.label().to_owned(),
                ));
            }
            let asset = item
                .asset_account_id
                .map_or_else(|| self.role(AccountRole::InventoryAsset), Ok)?;
            let p = self.line_posting(line, asset, line.amount);
            p.quantity = Some(line.quantity_change.unwrap_or(Decimal::ZERO));
            total = total
                .checked_add(line.amount)
                .ok_or(PostingError::Overflow)?;
        }
        let mut a = Posting::new(adj, -total);
        a.memo.clone_from(&self.txn.memo);
        a.class_id = self.txn.class_id;
        self.postings.push(a);
        self.total = total;
        Ok(())
    }

    /// Build assembly: component lines take value out of their asset accounts,
    /// the assembly line puts the same value in. Lines' amounts must sum to zero.
    fn build_assembly(&mut self) -> Result<(), PostingError> {
        let mut built = Money::ZERO;
        for line in &self.txn.lines {
            if line.kind == LineKind::Text {
                continue;
            }
            let item_id = line
                .item_id
                .ok_or(PostingError::UnknownItem(ItemId::default()))?;
            let item = self.item(item_id)?;
            let asset = item
                .asset_account_id
                .map_or_else(|| self.role(AccountRole::InventoryAsset), Ok)?;
            let p = self.line_posting(line, asset, line.amount);
            p.quantity = Some(line.quantity_change.unwrap_or(Decimal::ZERO));
            if line.amount.is_positive() {
                built += line.amount;
            }
        }
        self.total = built;
        Ok(())
    }
}

trait ApplyQty {
    fn apply_qty(self, q: Decimal) -> Decimal;
}

impl ApplyQty for Direction {
    fn apply_qty(self, q: Decimal) -> Decimal {
        match self {
            Direction::Normal => q,
            Direction::Reversed => -q,
        }
    }
}

/// Where an item posts on a purchase form. QBD's one-sided items post purchases
/// to their only account; two-sided items to the purchase (expense) account.
fn purchase_account(item: &Item) -> Option<AccountId> {
    if item.has_purchase_info || item.item_type == ItemType::FixedAsset {
        item.expense_account_id
            .or(item.asset_account_id)
            .or(item.income_account_id)
    } else {
        item.income_account_id.or(item.expense_account_id)
    }
}

/// Drop zero-amount postings that carry nothing a register would show, unless the
/// whole transaction is zero (a $0 check still appears in its register).
fn merge_zero_lines(postings: Vec<Posting>) -> Vec<Posting> {
    if postings.iter().all(|p| p.amount.is_zero()) {
        return postings;
    }
    postings
        .into_iter()
        .filter(|p| {
            !p.amount.is_zero() || p.quantity.is_some_and(|q| !q.is_zero()) || p.line_id.is_none()
        })
        .collect()
}

/// Sum postings per account, debit-positive: the core of every balance.
pub fn sum_by_account(postings: &[Posting]) -> BTreeMap<AccountId, Money> {
    let mut out = BTreeMap::new();
    for p in postings {
        *out.entry(p.account_id).or_insert(Money::ZERO) += p.amount;
    }
    out
}
