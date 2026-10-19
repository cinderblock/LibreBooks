//! The one shape every source becomes.
//!
//! An OFX file, a CSV export, a SimpleFIN download and a Direct Connect response
//! all turn into [`Statement`]s: an account, its balances, and its transactions
//! with the bank's id for each. Everything after parsing (dedupe, rules,
//! matching, adding) only ever sees this shape, so a new source is just a new
//! parser.
//!
//! Sign convention: an amount is **positive when money goes into the account**,
//! from the account holder's side. A deposit is positive and a check negative; on
//! a credit card a purchase is negative and a payment or refund positive. That
//! is OFX's and SimpleFIN's convention, and it equals the debit-positive posting
//! the matching register line has in that account.

use std::collections::HashMap;

use jiff::civil::Date;
use lb_core::Money;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::text;

/// What kind of account a statement is for, as the source describes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AccountKind {
    #[default]
    Checking,
    Savings,
    MoneyMarket,
    /// A line of credit (OFX `CREDITLINE`).
    CreditLine,
    CreditCard,
    /// A certificate of deposit, a loan, or a source that doesn't say.
    Other,
}

impl AccountKind {
    /// OFX's `ACCTTYPE` values.
    pub fn from_ofx(s: &str) -> AccountKind {
        match s.trim().to_ascii_uppercase().as_str() {
            "CHECKING" => AccountKind::Checking,
            "SAVINGS" => AccountKind::Savings,
            "MONEYMRKT" => AccountKind::MoneyMarket,
            "CREDITLINE" => AccountKind::CreditLine,
            _ => AccountKind::Other,
        }
    }

    pub const fn ofx_name(self) -> &'static str {
        match self {
            AccountKind::Checking => "CHECKING",
            AccountKind::Savings => "SAVINGS",
            AccountKind::MoneyMarket => "MONEYMRKT",
            AccountKind::CreditLine => "CREDITLINE",
            AccountKind::CreditCard | AccountKind::Other => "CD",
        }
    }
}

/// The account a statement is for. The account number itself is never kept:
/// only its last four characters, and a fingerprint to recognise it next time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatementAccount {
    /// The routing number (OFX `BANKID`). Public, not a secret.
    pub bank_id: Option<String>,
    /// `****1234`. Empty when the source gives no account number (most CSVs).
    pub account_id_masked: String,
    /// A fingerprint of the bank id and full account number (or of the provider's
    /// account id), stable across downloads: how a file is matched to the
    /// LibreBooks account it was imported into last time. Empty when unknown.
    pub account_key: String,
    pub account_type: AccountKind,
    /// ISO 4217, when the source says.
    pub currency: Option<String>,
    /// The institution's name (OFX `ORG`, SimpleFIN `org.name`).
    pub institution: Option<String>,
    /// OFX financial institution id (`FID`).
    pub fid: Option<String>,
    /// Intuit's bank id from QFX/QBO files (`INTU.BID`).
    pub intu_bid: Option<String>,
    /// The account's name at the provider ("Everyday Checking").
    pub name: Option<String>,
    /// The provider's own id for the account (SimpleFIN, Plaid…), used to pick
    /// one account out of a multi-account download.
    pub remote_id: Option<String>,
}

impl StatementAccount {
    /// Fill in the masked number and fingerprint from a full account number.
    pub fn set_number(&mut self, bank_id: Option<&str>, account_number: &str) {
        self.account_id_masked = text::mask_account(account_number);
        self.account_key =
            fingerprint(&[bank_id.unwrap_or_default(), &text::squeeze(account_number)]);
    }
}

/// A short, stable, one-way fingerprint of some text parts.
pub fn fingerprint(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update(b"|");
        }
        h.update(p.trim().to_uppercase().as_bytes());
    }
    hex::encode(h.finalize())[..24].to_owned()
}

/// Which way money moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Direction {
    /// Into the account: a deposit, a card payment or refund.
    In,
    /// Out of the account: a check, a withdrawal, a card purchase.
    Out,
}

impl Direction {
    /// The direction of a feed amount (zero counts as in).
    pub fn of(amount: Money) -> Direction {
        if amount.is_negative() {
            Direction::Out
        } else {
            Direction::In
        }
    }
}

/// OFX's transaction types (`TRNTYPE`). Other sources map onto these where they
/// can and use [`TrnType::Other`] where they can't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TrnType {
    Credit,
    Debit,
    Interest,
    Dividend,
    Fee,
    ServiceCharge,
    Deposit,
    Atm,
    PointOfSale,
    Transfer,
    Check,
    Payment,
    Cash,
    DirectDeposit,
    DirectDebit,
    RepeatPayment,
    Hold,
    #[default]
    Other,
}

impl TrnType {
    pub fn from_ofx(s: &str) -> TrnType {
        match s.trim().to_ascii_uppercase().as_str() {
            "CREDIT" => TrnType::Credit,
            "DEBIT" => TrnType::Debit,
            "INT" => TrnType::Interest,
            "DIV" => TrnType::Dividend,
            "FEE" => TrnType::Fee,
            "SRVCHG" => TrnType::ServiceCharge,
            "DEP" => TrnType::Deposit,
            "ATM" => TrnType::Atm,
            "POS" => TrnType::PointOfSale,
            "XFER" => TrnType::Transfer,
            "CHECK" => TrnType::Check,
            "PAYMENT" => TrnType::Payment,
            "CASH" => TrnType::Cash,
            "DIRECTDEP" => TrnType::DirectDeposit,
            "DIRECTDEBIT" => TrnType::DirectDebit,
            "REPEATPMT" => TrnType::RepeatPayment,
            "HOLD" => TrnType::Hold,
            _ => TrnType::Other,
        }
    }

    /// The type OFX would give a transaction of this sign when the source says
    /// nothing more specific.
    pub fn by_sign(amount: Money) -> TrnType {
        if amount.is_negative() {
            TrnType::Debit
        } else {
            TrnType::Credit
        }
    }
}

/// One transaction on a statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatementTxn {
    /// The bank's id for this transaction (OFX `FITID`), or one synthesised from
    /// its date, amount and payee when the source has none (see
    /// [`synthesize_fitids`]).
    pub fitid: String,
    pub fitid_synthesized: bool,
    /// The date the bank posted it.
    pub posted: Date,
    /// Positive when money goes into the account.
    pub amount: Money,
    /// The name as the bank wrote it.
    pub payee: Option<String>,
    pub memo: Option<String>,
    pub check_number: Option<String>,
    pub trntype: TrnType,
    /// Everything the source said about it, verbatim.
    #[ts(type = "unknown")]
    pub raw: serde_json::Value,
}

impl StatementTxn {
    pub fn new(posted: Date, amount: Money) -> StatementTxn {
        StatementTxn {
            fitid: String::new(),
            fitid_synthesized: false,
            posted,
            amount,
            payee: None,
            memo: None,
            check_number: None,
            trntype: TrnType::by_sign(amount),
            raw: serde_json::Value::Null,
        }
    }
}

/// A statement: one account's transactions over a period, as one source gave them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Statement {
    pub account: StatementAccount,
    /// The balance the bank reports (OFX `LEDGERBAL`), in the account holder's
    /// sense: a card's balance owed is negative.
    pub ledger_balance: Option<Money>,
    pub available_balance: Option<Money>,
    pub balance_date: Option<Date>,
    pub start: Option<Date>,
    pub end: Option<Date>,
    pub transactions: Vec<StatementTxn>,
    /// What the parser had to skip or guess, in words a person can act on.
    pub warnings: Vec<String>,
}

impl Statement {
    /// Make the statement ready to import: synthesise missing ids, settle
    /// duplicate ids, and fill in the period from the transactions if the source
    /// didn't give one.
    pub fn finish(&mut self) {
        synthesize_fitids(&mut self.transactions);
        settle_duplicate_fitids(self);
        let first = self.transactions.iter().map(|t| t.posted).min();
        let last = self.transactions.iter().map(|t| t.posted).max();
        if self.start.is_none() {
            self.start = first;
        }
        if self.end.is_none() {
            self.end = last;
        }
    }

    /// Reverse every amount (and balance): for a source that has the signs
    /// backwards for this account.
    pub fn reverse_signs(&mut self) {
        for t in &mut self.transactions {
            t.amount = -t.amount;
        }
        self.ledger_balance = self.ledger_balance.map(|m| -m);
        self.available_balance = self.available_balance.map(|m| -m);
    }
}

/// Give every transaction without an id a stable one.
///
/// The id is `lb1:` + the first 16 hex digits of SHA-256 over
/// `date|amount|payee` (payee squeezed to upper-case letters and digits, memo when
/// there is no payee) + `:` + how many earlier transactions in this statement had
/// the same date, amount and payee. Importing the same file twice, or a later
/// file that overlaps it, gives the same ids, so the store's dedupe on
/// `(account, fitid)` works; two identical coffees on one day stay two (`:0`,
/// `:1`). If a bank later rewrites a description, the id changes and the
/// transaction shows up again, as a strong match for the copy already there.
pub fn synthesize_fitids(txns: &mut [StatementTxn]) {
    let mut seen: HashMap<String, u32> = HashMap::new();
    for t in txns.iter_mut().filter(|t| t.fitid.trim().is_empty()) {
        let who = t
            .payee
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .or(t.memo.as_deref())
            .unwrap_or_default();
        let key = format!("{}|{}|{}", t.posted, t.amount.minor(), text::squeeze(who));
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        let n = seen.entry(digest.clone()).or_insert(0);
        t.fitid = format!("lb1:{}:{n}", &digest[..16]);
        t.fitid_synthesized = true;
        *n += 1;
    }
}

/// Banks occasionally repeat an id within one statement. The same id on the
/// same date and amount is the same transaction listed twice and is dropped;
/// the same id on a different transaction gets a `#2`, `#3`… suffix so neither
/// is lost. Both are reported.
fn settle_duplicate_fitids(s: &mut Statement) {
    let mut seen: HashMap<String, (Date, Money, u32)> = HashMap::new();
    let mut keep = Vec::with_capacity(s.transactions.len());
    for mut t in std::mem::take(&mut s.transactions) {
        match seen.get_mut(&t.fitid) {
            None => {
                seen.insert(t.fitid.clone(), (t.posted, t.amount, 1));
                keep.push(t);
            }
            Some((date, amount, _)) if *date == t.posted && *amount == t.amount => {
                s.warnings.push(format!(
                    "transaction {} on {} for {} is listed twice; kept once",
                    t.fitid, t.posted, t.amount
                ));
            }
            Some((_, _, count)) => {
                *count += 1;
                let original = t.fitid.clone();
                t.fitid = format!("{original}#{count}");
                s.warnings.push(format!(
                    "the bank used id {original} for two different transactions; the one on {} for {} is kept as {}",
                    t.posted, t.amount, t.fitid
                ));
                keep.push(t);
            }
        }
    }
    s.transactions = keep;
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn txn(d: Date, cents: i64, payee: &str) -> StatementTxn {
        StatementTxn {
            payee: Some(payee.into()),
            ..StatementTxn::new(d, Money(cents))
        }
    }

    #[test]
    fn synthesised_ids_are_stable_and_keep_twins_apart() {
        let d = date(2026, 10, 1);
        let mut a = vec![
            txn(d, -450, "Blue Bottle"),
            txn(d, -450, "Blue Bottle"),
            txn(d, -1200, "Safeway"),
        ];
        let mut b = a.clone();
        synthesize_fitids(&mut a);
        synthesize_fitids(&mut b);
        assert_eq!(a, b);
        assert!(a[0].fitid.starts_with("lb1:") && a[0].fitid.ends_with(":0"));
        assert!(a[1].fitid.ends_with(":1"));
        assert_eq!(a[0].fitid[..20], a[1].fitid[..20]);
        assert_ne!(a[0].fitid, a[2].fitid);
        assert!(a.iter().all(|t| t.fitid_synthesized));

        // Case and punctuation in the payee don't change the id.
        let mut c = vec![txn(d, -1200, "SAFEWAY.")];
        synthesize_fitids(&mut c);
        assert_eq!(c[0].fitid, a[2].fitid);

        // Ids the bank gave are left alone.
        let mut given = vec![StatementTxn {
            fitid: "ABC".into(),
            ..txn(d, -1, "x")
        }];
        synthesize_fitids(&mut given);
        assert_eq!(given[0].fitid, "ABC");
        assert!(!given[0].fitid_synthesized);
    }

    #[test]
    fn duplicate_ids_are_settled() {
        let d = date(2026, 10, 1);
        let with_id = |id: &str, cents| StatementTxn {
            fitid: id.into(),
            ..txn(d, cents, "x")
        };
        let mut s = Statement {
            transactions: vec![with_id("1", -100), with_id("1", -100), with_id("1", -999)],
            ..Statement::default()
        };
        s.finish();
        let ids: Vec<&str> = s.transactions.iter().map(|t| t.fitid.as_str()).collect();
        assert_eq!(ids, vec!["1", "1#2"]);
        assert_eq!(s.warnings.len(), 2);
        assert_eq!(s.start, Some(d));
    }

    #[test]
    fn fingerprints_ignore_case_and_spacing() {
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&[" A", "B "]));
        assert_ne!(fingerprint(&["a", "b"]), fingerprint(&["ab"]));
        assert_eq!(fingerprint(&["x"]).len(), 24);
    }
}
