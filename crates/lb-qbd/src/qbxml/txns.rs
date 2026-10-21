//! Transactions: every type the bridge exposes, into [`lb_core::Txn`].
//!
//! How QBD's forms map:
//!
//! - **Lines.** Purchase forms (check, bill) have an expenses tab
//!   (`ExpenseLineRet` → [`LineKind::Account`]) and an items tab (`ItemLineRet` →
//!   [`LineKind::Item`], `ItemGroupLineRet` → the group's own line followed by its
//!   members). Sales forms have `…LineRet` and `…LineGroupRet`. A group's own
//!   line carries no amount (its members carry it; QBD's `TotalAmount` is kept
//!   in its `extra`), so nothing is counted twice. Lines are put back in the order
//!   they were entered using `TxnLineID` (see [`super::record::line_order`]).
//! - **Journal entries.** `JournalDebitLine` amounts are positive, `JournalCreditLine`
//!   negative (LibreBooks journal lines are debit-positive).
//! - **Payments.** Each `AppliedToTxnRet` is an [`Application`]: cash `Amount`,
//!   `DiscountAmount` with its account and class. Credits used against the same
//!   target appear among its `LinkedTxn` (alongside *other* payments to it, which
//!   are ignored here); each becomes a credit-only application that the sync
//!   engine de-duplicates across payments.
//! - **Deposits.** A `DepositLineRet` with a `TxnID` moves that payment out of
//!   Undeposited Funds (`linked_txn_id`); `CashBackInfoRet` is the cash-back line.
//! - **Time.** A `TimeTrackingRet` is a non-posting transaction with one line: the
//!   service item, hours as the quantity, the customer:job as the line's name.
//!
//! QBD's totals (`Amount`, `Subtotal` + `SalesTaxTotal`, `DepositTotal`…) and
//! open amounts (`BalanceRemaining`, `CreditRemaining`, `OpenAmount`,
//! `UnusedPayment`) are returned beside the transaction so the sync engine can
//! compare them with what LibreBooks' own posting rules compute.

use lb_core::{Application, BillableStatus, LineKind, Money, Txn, TxnLine, TxnType};
use rust_decimal::Decimal;
use serde_json::{Map, Value};

use super::lists::address;
use super::record::{QbRef, Record, line_order};
use super::{NormaliseError, Resolver, require};
use crate::entity::QbdEntity;

/// A normalised transaction and what to check it against.
#[derive(Clone, Debug, PartialEq)]
pub struct TxnImport {
    pub txn: Txn,
    /// The form's total as QBD stored it.
    pub qbd_total: Option<Money>,
    /// Sales tax as QBD stored it (sales forms).
    pub qbd_sales_tax: Option<Money>,
    /// What QBD says is still open on it (unpaid, unused).
    pub qbd_open_balance: Option<Money>,
    /// Links to transactions that aren't in LibreBooks.
    pub missing_links: Vec<MissingLink>,
    /// Sub-record fields with nowhere to go (address parts, applied-to details).
    pub unmapped: Vec<String>,
    pub notes: Vec<String>,
}

/// A link the transaction makes that couldn't be imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingLink {
    /// What the link is: `payment applied to`, `credit used`, `deposit of`.
    pub role: &'static str,
    pub txn_type: Option<String>,
    pub txn_id: String,
    /// The bridge exposes this type, so it may simply not be imported yet (a
    /// later sync can fill the link in). `false`: it never will be.
    pub exposed: bool,
}

impl MissingLink {
    fn new(role: &'static str, txn_type: Option<String>, txn_id: String) -> MissingLink {
        let exposed = txn_type
            .as_deref()
            .is_none_or(|t| QbdEntity::for_qbxml_txn_type(t).is_some());
        MissingLink {
            role,
            txn_type,
            txn_id,
            exposed,
        }
    }
}

impl std::fmt::Display for MissingLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} {}",
            self.role,
            self.txn_type.as_deref().unwrap_or("transaction"),
            self.txn_id
        )?;
        if self.exposed {
            f.write_str(" (not imported yet)")
        } else {
            f.write_str(" (the bridge doesn't expose that type)")
        }
    }
}

/// Normalise one complete transaction record (with its lines; the bridge's list
/// rows leave them out).
pub fn txn(
    entity: QbdEntity,
    v: &Value,
    res: &mut dyn Resolver,
) -> Result<TxnImport, NormaliseError> {
    let txn_type = entity.txn_type().ok_or_else(|| {
        NormaliseError::Unsupported(format!("{} isn't a transaction", entity.name()))
    })?;
    let mut r = Record::new(entity.name(), v)?;
    let mut n = Normaliser {
        res,
        imp: TxnImport {
            txn: header(&mut r, txn_type)?,
            qbd_total: None,
            qbd_sales_tax: None,
            qbd_open_balance: None,
            missing_links: Vec::new(),
            unmapped: Vec::new(),
            notes: Vec::new(),
        },
        blocks: Vec::new(),
    };
    match entity {
        QbdEntity::Check => n.check(&mut r)?,
        QbdEntity::Bill => n.bill(&mut r)?,
        QbdEntity::Invoice
        | QbdEntity::CreditMemo
        | QbdEntity::SalesReceipt
        | QbdEntity::Estimate => n.sale(entity, &mut r)?,
        QbdEntity::PurchaseOrder => n.purchase_order(&mut r)?,
        QbdEntity::JournalEntry => n.journal(&mut r)?,
        QbdEntity::Deposit => n.deposit(&mut r)?,
        QbdEntity::ReceivePayment => n.receive_payment(&mut r)?,
        QbdEntity::BillPaymentCheck => n.bill_payment(&mut r)?,
        QbdEntity::TimeTracking => n.time(&mut r)?,
        other => {
            return Err(NormaliseError::Unsupported(format!(
                "{} isn't a transaction",
                other.name()
            )));
        }
    }
    let Normaliser {
        mut imp, blocks, ..
    } = n;
    let mut blocks = blocks;
    blocks.sort_by_key(|(order, _)| *order);
    imp.txn.lines = blocks.into_iter().flat_map(|(_, l)| l).collect();
    // QBD has no void flag: a voided transaction keeps its lines at zero and gets
    // "VOID:" in its memo.
    if imp
        .txn
        .memo
        .as_deref()
        .is_some_and(|m| m.starts_with("VOID:"))
        && imp.qbd_total.is_none_or(Money::is_zero)
        && imp.txn.lines.iter().all(|l| l.amount.is_zero())
        && imp.txn.applications.iter().all(|a| a.amount.is_zero())
    {
        imp.txn.is_void = true;
    }
    imp.txn.extra = object_or_none(r.leftovers());
    Ok(imp)
}

fn object_or_none(m: Map<String, Value>) -> Option<Value> {
    (!m.is_empty()).then_some(Value::Object(m))
}

/// Fields every transaction has.
fn header(r: &mut Record<'_>, txn_type: TxnType) -> Result<Txn, NormaliseError> {
    // The sync engine reads the id, edit sequence and modified time itself.
    r.ignore(&["TxnID", "TimeModified", "EditSequence"]);
    let date = r.required_date("TxnDate")?;
    let mut t = Txn::new(txn_type, date);
    t.created_at = r.timestamp("TimeCreated")?;
    t.number = r.text("RefNumber");
    t.memo = r.text("Memo");
    Ok(t)
}

fn billable(s: Option<&str>) -> BillableStatus {
    match s {
        Some("Billable") => BillableStatus::Billable,
        Some("HasBeenBilled") => BillableStatus::HasBeenBilled,
        _ => BillableStatus::NotBillable,
    }
}

/// A line's leftovers plus its QBD line id, for `TxnLine::extra`. The sync engine
/// reuses LibreBooks line ids by matching `TxnLineID` on re-import.
fn line_extra(l: &Record<'_>, txn_line_id: Option<&str>) -> Option<Value> {
    let mut m = l.leftovers();
    if let Some(id) = txn_line_id {
        m.insert("TxnLineID".into(), Value::String(id.to_owned()));
    }
    object_or_none(m)
}

/// The QBD `TxnLineID` stored on a LibreBooks line by this importer.
pub fn qbd_line_id(line: &TxnLine) -> Option<&str> {
    line.extra.as_ref()?.get("TxnLineID")?.as_str()
}

/// Which side of the books a form's item lines are on: purchase lines carry a
/// `Cost` and may name a customer:job; sales lines a `Rate` or `RatePercent`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Purchase,
    Sale,
}

struct Normaliser<'r> {
    res: &'r mut dyn Resolver,
    imp: TxnImport,
    /// Lines in groups that keep together (a group and its members), each with
    /// the sort key of its first `TxnLineID`.
    blocks: Vec<(u64, Vec<TxnLine>)>,
}

impl Normaliser<'_> {
    fn txn(&mut self) -> &mut Txn {
        &mut self.imp.txn
    }

    fn opt_account(
        &mut self,
        what: &'static str,
        r: Option<QbRef>,
    ) -> Result<Option<lb_core::AccountId>, NormaliseError> {
        match r {
            Some(a) => require(what, &a, self.res.account(&a)).map(Some),
            None => Ok(None),
        }
    }

    fn opt_entity(
        &mut self,
        what: &'static str,
        r: Option<QbRef>,
    ) -> Result<Option<lb_core::EntityId>, NormaliseError> {
        match r {
            Some(e) => require(what, &e, self.res.entity(&e)).map(Some),
            None => Ok(None),
        }
    }

    fn opt_class(&mut self, r: Option<QbRef>) -> Result<Option<lb_core::ClassId>, NormaliseError> {
        match r {
            Some(c) => require("class", &c, self.res.class(&c)).map(Some),
            None => Ok(None),
        }
    }

    fn opt_tax_code(
        &mut self,
        r: Option<QbRef>,
    ) -> Result<Option<lb_core::SalesTaxCodeId>, NormaliseError> {
        match r {
            Some(c) => self.res.sales_tax_code(&c),
            None => Ok(None),
        }
    }

    fn opt_payment_method(
        &mut self,
        r: Option<QbRef>,
    ) -> Result<Option<lb_core::PaymentMethodId>, NormaliseError> {
        match r {
            Some(p) => require("payment method", &p, self.res.payment_method(&p)).map(Some),
            None => Ok(None),
        }
    }

    fn address(
        &mut self,
        r: &mut Record<'_>,
        key: &str,
    ) -> Result<lb_core::Address, NormaliseError> {
        address(r, key, &mut self.imp.unmapped)
    }

    /// Terms, due date, class, sales rep and printing flags: the parts of a
    /// header that several forms share.
    fn common(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let class = self.opt_class(r.reference("ClassRef"))?;
        let terms = match r.reference("TermsRef") {
            Some(t) => Some(require("terms", &t, self.res.terms(&t))?),
            None => None,
        };
        let rep = match r.reference("SalesRepRef") {
            Some(s) => Some(require("sales rep", &s, self.res.sales_rep(&s))?),
            None => None,
        };
        let t = self.txn();
        t.class_id = class;
        t.terms_id = terms;
        t.sales_rep_id = rep;
        t.due_date = r.date("DueDate")?;
        t.ship_date = r.date("ShipDate")?;
        t.po_number = r.text("PONumber");
        t.is_to_be_printed = r.bool("IsToBePrinted")?.unwrap_or(false);
        t.is_to_be_emailed = r.bool("IsToBeEmailed")?.unwrap_or(false);
        t.is_pending = r.bool("IsPending")?.unwrap_or(false);
        Ok(())
    }

    fn push(&mut self, order: u64, lines: Vec<TxnLine>) {
        self.blocks.push((order, lines));
    }

    // ---- lines ----

    fn expense_line(&mut self, mut l: Record<'_>) -> Result<(), NormaliseError> {
        let id = l.text("TxnLineID");
        let amount = l.money("Amount")?.unwrap_or(Money::ZERO);
        let account = l.reference("AccountRef");
        let mut line = match account {
            Some(a) => TxnLine::account(require("account", &a, self.res.account(&a))?, amount),
            None if amount.is_zero() => TxnLine::new(LineKind::Text),
            None => {
                return Err(NormaliseError::Unsupported(
                    "an expense line with an amount but no account".into(),
                ));
            }
        };
        line.description = l.text("Memo");
        line.entity_id = self.opt_entity("customer", l.reference("CustomerRef"))?;
        line.class_id = self.opt_class(l.reference("ClassRef"))?;
        line.sales_tax_code_id = self.opt_tax_code(l.reference("SalesTaxCodeRef"))?;
        line.billable = billable(l.text("BillableStatus").as_deref());
        line.extra = line_extra(&l, id.as_deref());
        self.push(line_order(id.as_deref()), vec![line]);
        Ok(())
    }

    /// An item line on any form. Lines with no item are description-only text
    /// lines.
    fn item_line(&mut self, mut l: Record<'_>, side: Side) -> Result<TxnLine, NormaliseError> {
        let id = l.text("TxnLineID");
        let item = l.reference("ItemRef");
        let amount = l.money("Amount")?;
        let mut line = match item {
            Some(i) => {
                let mut line = TxnLine::new(LineKind::Item);
                line.item_id = Some(require("item", &i, self.res.item(&i))?);
                line
            }
            None if amount.is_none_or(Money::is_zero) => TxnLine::new(LineKind::Text),
            None => {
                return Err(NormaliseError::Unsupported(
                    "an item line with an amount but no item".into(),
                ));
            }
        };
        line.amount = amount.unwrap_or(Money::ZERO);
        line.description = l.text("Desc");
        line.quantity = l.decimal("Quantity")?;
        line.unit_of_measure = l.text("UnitOfMeasure");
        if let Some(p) = l.decimal("RatePercent")? {
            line.rate = Some(p);
            line.rate_is_percent = true;
        } else if let Some(c) = l.decimal("Cost")? {
            line.rate = Some(c);
        } else {
            line.rate = l.decimal("Rate")?;
        }
        line.class_id = self.opt_class(l.reference("ClassRef"))?;
        line.sales_tax_code_id = self.opt_tax_code(l.reference("SalesTaxCodeRef"))?;
        line.service_date = l.date("ServiceDate")?;
        line.other1 = l.text("Other1");
        line.other2 = l.text("Other2");
        if side == Side::Purchase {
            line.entity_id = self.opt_entity("customer", l.reference("CustomerRef"))?;
            line.billable = billable(l.text("BillableStatus").as_deref());
        }
        // Order lines: QBD's "Clsd" mark, and a line QBD shows fully received or
        // invoiced. We can't link the receipts and invoices QBD counted, so a
        // fulfilled line is closed on import; the quantities stay in `extra`.
        let manually = l.bool("IsManuallyClosed")?.unwrap_or(false);
        let drawn = match l.peek("ReceivedQuantity").or_else(|| l.peek("Invoiced")) {
            Some(v) => v
                .as_str()
                .and_then(|t| t.trim().parse::<Decimal>().ok())
                .or_else(|| v.as_f64().and_then(Decimal::from_f64_retain)),
            None => None,
        };
        let fulfilled = match (drawn, line.quantity) {
            (Some(d), Some(q)) => q > Decimal::ZERO && d >= q,
            _ => false,
        };
        line.is_closed = manually || fulfilled;
        line.extra = line_extra(&l, id.as_deref());
        Ok(line)
    }

    /// A group line and its members, kept together.
    fn group(
        &mut self,
        mut g: Record<'_>,
        member_key: &str,
        side: Side,
    ) -> Result<(), NormaliseError> {
        let id = g.text("TxnLineID");
        let group_ref = g.reference("ItemGroupRef");
        let resolved = match &group_ref {
            Some(r) => self.res.item(r)?,
            None => None,
        };
        let mut head = match resolved {
            Some(item) => {
                let mut l = TxnLine::new(LineKind::Item);
                l.item_id = Some(item);
                l
            }
            // QBD's member-less reimbursable-expense group isn't an item in
            // LibreBooks; its line becomes text.
            None => TxnLine::new(LineKind::Text),
        };
        head.description = g
            .text("Desc")
            .or_else(|| group_ref.as_ref().and_then(|r| r.full_name.clone()));
        head.quantity = g.decimal("Quantity")?;
        head.unit_of_measure = g.text("UnitOfMeasure");
        let mut members: Vec<(u64, TxnLine)> = Vec::new();
        for m in g.children(member_key)? {
            let line = self.item_line(m, side)?;
            members.push((line_order(qbd_line_id(&line)), line));
        }
        members.sort_by_key(|(o, _)| *o);
        let mut extra = g.leftovers();
        if let Some(r) = &group_ref
            && resolved.is_none()
        {
            extra.insert(
                "ItemGroupRef".into(),
                serde_json::json!({"ListID": r.list_id, "FullName": r.full_name}),
            );
        }
        if let Some(id) = &id {
            extra.insert("TxnLineID".into(), Value::String(id.clone()));
        }
        head.extra = object_or_none(extra);
        let mut block = vec![head];
        block.extend(members.into_iter().map(|(_, l)| l));
        self.push(line_order(id.as_deref()), block);
        Ok(())
    }

    fn purchase_lines(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        for l in r.children("ExpenseLineRet")? {
            self.expense_line(l)?;
        }
        for l in r.children("ItemLineRet")? {
            let line = self.item_line(l, Side::Purchase)?;
            let order = line_order(qbd_line_id(&line));
            self.push(order, vec![line]);
        }
        for g in r.children("ItemGroupLineRet")? {
            self.group(g, "ItemLineRet", Side::Purchase)?;
        }
        Ok(())
    }

    fn sales_lines(
        &mut self,
        r: &mut Record<'_>,
        line_key: &str,
        group_key: &str,
        side: Side,
    ) -> Result<(), NormaliseError> {
        for l in r.children(line_key)? {
            let line = self.item_line(l, side)?;
            let order = line_order(qbd_line_id(&line));
            self.push(order, vec![line]);
        }
        for g in r.children(group_key)? {
            self.group(g, line_key, side)?;
        }
        Ok(())
    }

    // ---- forms ----

    fn check(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let bank = r.reference("AccountRef");
        let bank = match bank {
            Some(b) => require("bank account", &b, self.res.account(&b))?,
            None => return Err(missing("Check.AccountRef")),
        };
        let payee = self.opt_entity("payee", r.reference("PayeeEntityRef"))?;
        let addr = self.address(r, "Address")?;
        self.common(r)?;
        let amount = r.money("Amount")?;
        let t = self.txn();
        t.account_id = Some(bank);
        t.entity_id = payee;
        t.bill_address = addr;
        if let Some(a) = amount {
            t.amount = a;
        }
        self.imp.qbd_total = amount;
        self.purchase_lines(r)
    }

    fn bill(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let vendor = self.opt_entity("vendor", r.reference("VendorRef"))?;
        let ap = self.opt_account("A/P account", r.reference("APAccountRef"))?;
        let addr = self.address(r, "VendorAddress")?;
        self.common(r)?;
        self.imp.qbd_total = r.money("AmountDue")?;
        self.imp.qbd_open_balance = r.money("OpenAmount")?;
        r.ignore(&["IsPaid"]);
        let t = self.txn();
        t.entity_id = vendor;
        t.ar_ap_account_id = ap;
        t.bill_address = addr;
        self.purchase_lines(r)
    }

    fn purchase_order(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let vendor = self.opt_entity("vendor", r.reference("VendorRef"))?;
        let bill_to = self.address(r, "VendorAddress")?;
        let ship_to = self.address(r, "ShipAddress")?;
        self.common(r)?;
        let total = r.money("TotalAmount")?;
        self.imp.qbd_total = total;
        let message = r.reference("VendorMsg").and_then(|m| m.full_name);
        // QBD's "Closed" stamp; "fully received" closes every line instead (the
        // receipts themselves aren't linked by the import).
        let closed = r.bool("IsManuallyClosed")?.unwrap_or(false);
        let fully_received = r.bool("IsFullyReceived")?.unwrap_or(false);
        let t = self.txn();
        t.entity_id = vendor;
        t.bill_address = bill_to;
        t.ship_address = ship_to;
        t.message = message;
        t.is_manually_closed = closed;
        if let Some(a) = total {
            t.amount = a;
        }
        self.sales_lines(
            r,
            "PurchaseOrderLineRet",
            "PurchaseOrderLineGroupRet",
            Side::Purchase,
        )?;
        if fully_received {
            self.close_every_line();
        }
        Ok(())
    }

    /// Mark every item line closed (an order QBD reports fully received or
    /// invoiced, whose receipts and invoices the import can't link).
    fn close_every_line(&mut self) {
        for (_, lines) in &mut self.blocks {
            for l in lines.iter_mut().filter(|l| l.kind == LineKind::Item) {
                l.is_closed = true;
            }
        }
    }

    fn sale(&mut self, entity: QbdEntity, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let customer = match r.reference("CustomerRef") {
            Some(c) => require("customer", &c, self.res.entity(&c))?,
            None => return Err(missing(&format!("{}.CustomerRef", entity.name()))),
        };
        let ar = self.opt_account("A/R account", r.reference("ARAccountRef"))?;
        let bill_to = self.address(r, "BillAddress")?;
        let ship_to = self.address(r, "ShipAddress")?;
        self.common(r)?;
        let message = r.reference("CustomerMsgRef").and_then(|m| m.full_name);
        // Sales tax: the item (or group) charged and the customer's code. The
        // bridge doesn't list sales tax items, so the resolver creates them from
        // here, at the rate this form used.
        let rate = r.decimal("SalesTaxPercentage")?;
        let tax_item = match r.reference("ItemSalesTaxRef") {
            Some(i) => self.res.sales_tax_item(&i, rate)?,
            None => None,
        };
        let customer_code = self.opt_tax_code(r.reference("CustomerSalesTaxCodeRef"))?;
        self.imp.qbd_sales_tax = r.money("SalesTaxTotal")?;
        let subtotal = r.money("Subtotal")?;
        let total = r.money("TotalAmount")?;
        let tax = self.imp.qbd_sales_tax.unwrap_or(Money::ZERO);
        self.imp.qbd_total = total.or_else(|| subtotal.map(|s| s + tax));
        self.imp.qbd_open_balance = match entity {
            QbdEntity::Invoice => r.money("BalanceRemaining")?,
            QbdEntity::CreditMemo => r.money("CreditRemaining")?,
            _ => None,
        };
        r.ignore(&["AppliedAmount", "IsPaid"]);
        let (deposit_to, method) = if entity == QbdEntity::SalesReceipt {
            let d = self.opt_account("deposit account", r.reference("DepositToAccountRef"))?;
            let m = self.opt_payment_method(r.reference("PaymentMethodRef"))?;
            (d, m)
        } else {
            (None, None)
        };
        let t = self.txn();
        t.entity_id = Some(customer);
        t.ar_ap_account_id = ar;
        t.bill_address = bill_to;
        t.ship_address = ship_to;
        t.message = message;
        t.sales_tax_item_id = tax_item;
        t.customer_tax_code_id = customer_code;
        t.account_id = deposit_to;
        t.payment_method_id = method;
        if entity == QbdEntity::Estimate {
            if let Some(a) = self.imp.qbd_total {
                // Estimates don't post; keep QBD's total as the form's.
                self.imp.txn.amount = a;
            }
            // An inactive estimate is QBD's closed one.
            if r.bool("IsActive")? == Some(false) {
                self.imp.txn.is_manually_closed = true;
            }
        }
        let (line_key, group_key) = match entity {
            QbdEntity::Invoice => ("InvoiceLineRet", "InvoiceLineGroupRet"),
            QbdEntity::CreditMemo => ("CreditMemoLineRet", "CreditMemoLineGroupRet"),
            QbdEntity::SalesReceipt => ("SalesReceiptLineRet", "SalesReceiptLineGroupRet"),
            _ => ("EstimateLineRet", "EstimateLineGroupRet"),
        };
        self.sales_lines(r, line_key, group_key, Side::Sale)
    }

    fn journal(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        self.txn().is_adjustment = r.bool("IsAdjustment")?.unwrap_or(false);
        for (key, sign) in [("JournalDebitLine", 1), ("JournalCreditLine", -1)] {
            for mut l in r.children(key)? {
                let id = l.text("TxnLineID");
                let Some(a) = l.reference("AccountRef") else {
                    return Err(missing(&format!("JournalEntry.{key}.AccountRef")));
                };
                let account = require("account", &a, self.res.account(&a))?;
                let amount = l.money("Amount")?.unwrap_or(Money::ZERO);
                let mut line = TxnLine::journal(account, if sign > 0 { amount } else { -amount });
                line.description = l.text("Memo");
                line.entity_id = self.opt_entity("name", l.reference("EntityRef"))?;
                line.class_id = self.opt_class(l.reference("ClassRef"))?;
                line.billable = billable(l.text("BillableStatus").as_deref());
                line.extra = line_extra(&l, id.as_deref());
                self.push(line_order(id.as_deref()), vec![line]);
            }
        }
        Ok(())
    }

    fn deposit(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let bank = match r.reference("DepositToAccountRef") {
            Some(b) => require("deposit account", &b, self.res.account(&b))?,
            None => return Err(missing("Deposit.DepositToAccountRef")),
        };
        self.txn().account_id = Some(bank);
        self.txn().class_id = self.opt_class(r.reference("ClassRef"))?;
        self.imp.qbd_total = r.money("DepositTotal")?;
        for mut l in r.children("DepositLineRet")? {
            let id = l.text("TxnLineID");
            let Some(a) = l.reference("AccountRef") else {
                return Err(missing("Deposit.DepositLineRet.AccountRef"));
            };
            let account = require("account", &a, self.res.account(&a))?;
            let mut line = TxnLine::deposit(account, l.money("Amount")?.unwrap_or(Money::ZERO));
            line.entity_id = self.opt_entity("name", l.reference("EntityRef"))?;
            line.description = l.text("Memo");
            line.check_number = l.text("CheckNumber");
            line.payment_method_id = self.opt_payment_method(l.reference("PaymentMethodRef"))?;
            line.class_id = self.opt_class(l.reference("ClassRef"))?;
            let linked_type = l.text("TxnType");
            if let Some(linked) = l.text("TxnID") {
                if let Some(t) = self.res.txn(linked_type.as_deref(), &linked)? {
                    line.linked_txn_id = Some(t);
                } else {
                    self.imp.missing_links.push(MissingLink::new(
                        "deposit of",
                        linked_type.clone(),
                        linked.clone(),
                    ));
                    // Keep QBD's link on the line so a later sync can see it.
                    let mut extra = l.leftovers();
                    extra.insert("TxnID".into(), Value::String(linked));
                    if let Some(t) = &linked_type {
                        extra.insert("TxnType".into(), Value::String(t.clone()));
                    }
                    if let Some(id) = &id {
                        extra.insert("TxnLineID".into(), Value::String(id.clone()));
                    }
                    line.extra = object_or_none(extra);
                    self.push(line_order(id.as_deref()), vec![line]);
                    continue;
                }
            }
            line.extra = line_extra(&l, id.as_deref());
            self.push(line_order(id.as_deref()), vec![line]);
        }
        if let Some(mut c) = r.child("CashBackInfoRet")? {
            let id = c.text("TxnLineID");
            let Some(a) = c.reference("AccountRef") else {
                return Err(missing("Deposit.CashBackInfoRet.AccountRef"));
            };
            let mut line = TxnLine::new(LineKind::CashBack);
            line.account_id = Some(require("cash back account", &a, self.res.account(&a))?);
            line.amount = c.money("Amount")?.unwrap_or(Money::ZERO);
            line.description = c.text("Memo");
            line.extra = line_extra(&c, id.as_deref());
            // Cash back always shows last.
            self.push(u64::MAX, vec![line]);
        }
        Ok(())
    }

    /// `AppliedToTxnRet`s → applications. `credit_types` are the `LinkedTxn` types
    /// that are credits used against the target.
    fn applications(
        &mut self,
        r: &mut Record<'_>,
        kind: &str,
        credit_types: &[&str],
    ) -> Result<(), NormaliseError> {
        for mut a in r.children("AppliedToTxnRet")? {
            let target_id = a.required_text("TxnID")?;
            let target_type = a.text("TxnType");
            let amount = a.money("Amount")?.unwrap_or(Money::ZERO);
            let discount = a.money("DiscountAmount")?.unwrap_or(Money::ZERO);
            let discount_account =
                self.opt_account("discount account", a.reference("DiscountAccountRef"))?;
            let discount_class = self.opt_class(a.reference("DiscountClassRef"))?;
            a.ignore(&["TxnDate", "RefNumber", "BalanceRemaining"]);
            let mut credits = Vec::new();
            for mut l in a.children("LinkedTxn")? {
                let t = l.text("TxnType");
                if !t.as_deref().is_some_and(|t| credit_types.contains(&t)) {
                    // Another payment (or anything else) linked to the same target.
                    continue;
                }
                let Some(credit_id) = l.text("TxnID") else {
                    continue;
                };
                let used = l.money("Amount")?.unwrap_or(Money::ZERO).abs();
                match self.res.txn(t.as_deref(), &credit_id)? {
                    Some(c) => credits.push((c, used)),
                    None => {
                        self.imp
                            .missing_links
                            .push(MissingLink::new("credit used", t, credit_id));
                    }
                }
            }
            for k in a.leftover_keys() {
                self.imp
                    .unmapped
                    .push(format!("{kind}.AppliedToTxnRet.{k}"));
            }
            let Some(target) = self.res.txn(target_type.as_deref(), &target_id)? else {
                self.imp.missing_links.push(MissingLink::new(
                    "payment applied to",
                    target_type,
                    target_id,
                ));
                continue;
            };
            self.imp.txn.applications.push(Application {
                discount,
                discount_account_id: discount_account,
                discount_class_id: discount_class,
                ..Application::cash(target, amount)
            });
            for (credit, used) in credits {
                self.imp.txn.applications.push(Application {
                    credit_txn_id: Some(credit),
                    credit_amount: used,
                    ..Application::cash(target, Money::ZERO)
                });
            }
        }
        Ok(())
    }

    fn receive_payment(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let customer = match r.reference("CustomerRef") {
            Some(c) => require("customer", &c, self.res.entity(&c))?,
            None => return Err(missing("ReceivePayment.CustomerRef")),
        };
        let ar = self.opt_account("A/R account", r.reference("ARAccountRef"))?;
        let deposit_to = self.opt_account("deposit account", r.reference("DepositToAccountRef"))?;
        let method = self.opt_payment_method(r.reference("PaymentMethodRef"))?;
        let amount = r.money("TotalAmount")?.unwrap_or(Money::ZERO);
        self.imp.qbd_total = Some(amount);
        self.imp.qbd_open_balance = r.money("UnusedPayment")?;
        // Card details stay in QuickBooks.
        r.ignore(&["UnusedCredits", "CreditCardTxnInfo"]);
        let t = self.txn();
        t.entity_id = Some(customer);
        t.ar_ap_account_id = ar;
        t.account_id = deposit_to;
        t.payment_method_id = method;
        t.amount = amount;
        self.applications(r, "ReceivePayment", &["CreditMemo"])
    }

    fn bill_payment(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let vendor = match r.reference("PayeeEntityRef") {
            Some(v) => require("vendor", &v, self.res.entity(&v))?,
            None => return Err(missing("BillPaymentCheck.PayeeEntityRef")),
        };
        let ap = self.opt_account("A/P account", r.reference("APAccountRef"))?;
        let bank = self.opt_account("bank account", r.reference("BankAccountRef"))?;
        let addr = self.address(r, "Address")?;
        let amount = r.money("Amount")?.unwrap_or(Money::ZERO);
        self.imp.qbd_total = Some(amount);
        let printed = r.bool("IsToBePrinted")?.unwrap_or(false);
        let t = self.txn();
        t.entity_id = Some(vendor);
        t.ar_ap_account_id = ap;
        t.account_id = bank;
        t.bill_address = addr;
        t.amount = amount;
        t.is_to_be_printed = printed;
        self.applications(r, "BillPaymentCheck", &["VendorCredit"])
    }

    fn time(&mut self, r: &mut Record<'_>) -> Result<(), NormaliseError> {
        let who = self.opt_entity("name", r.reference("EntityRef"))?;
        self.txn().entity_id = who;
        let item = match r.reference("ItemServiceRef") {
            Some(i) => Some(require("service item", &i, self.res.item(&i))?),
            None => None,
        };
        let mut line = TxnLine::new(if item.is_some() {
            LineKind::Item
        } else {
            LineKind::Text
        });
        line.item_id = item;
        line.entity_id = self.opt_entity("customer", r.reference("CustomerRef"))?;
        line.class_id = self.opt_class(r.reference("ClassRef"))?;
        line.description = r.text("Notes");
        line.billable = billable(r.text("BillableStatus").as_deref());
        if line.billable == BillableStatus::NotBillable && r.bool("IsBillable")? == Some(true) {
            line.billable = BillableStatus::Billable;
        }
        let mut extra = Map::new();
        if let Some(d) = r.text("Duration") {
            line.quantity = Some(duration_hours(&d).ok_or_else(|| super::FieldError {
                field: "TimeTracking.Duration".into(),
                value: d.clone(),
                expected: "a duration like PT1H30M",
            })?);
            extra.insert("Duration".into(), Value::String(d));
        }
        if let Some(p) = r.raw("PayrollItemWageRef") {
            extra.insert("PayrollItemWageRef".into(), p.clone());
        }
        line.extra = object_or_none(extra);
        self.push(0, vec![line]);
        Ok(())
    }
}

fn missing(field: &str) -> NormaliseError {
    NormaliseError::Field(super::FieldError {
        field: field.to_owned(),
        value: String::new(),
        expected: "present",
    })
}

/// QBD's `Duration` (`PT8H0M`, `PT0H35M`, `PT1H30M15S`) in hours, to six places.
pub fn duration_hours(s: &str) -> Option<Decimal> {
    let rest = s.strip_prefix("PT")?;
    let mut total = Decimal::ZERO;
    let mut num = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: Decimal = num.parse().ok()?;
        num.clear();
        total += match c {
            'H' => n,
            'M' => n / Decimal::from(60),
            'S' => n / Decimal::from(3600),
            _ => return None,
        };
    }
    num.is_empty().then(|| total.round_dp(6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_become_hours() {
        assert_eq!(duration_hours("PT8H0M"), Some(Decimal::from(8)));
        assert_eq!(duration_hours("PT0H30M"), Some(Decimal::new(5, 1)));
        assert_eq!(duration_hours("PT0H35M"), Some(Decimal::new(583_333, 6)));
        assert_eq!(duration_hours("PT1H30M36S"), Some(Decimal::new(151, 2)));
        assert_eq!(duration_hours("8 hours"), None);
        assert_eq!(duration_hours("PT8"), None);
    }
}
