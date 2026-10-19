//! The ledger as reports read it, on either basis.
//!
//! Every report is built from *facts*: a posting's amount in an account on a
//! date, with its name, class, item and transaction. On the accrual basis the
//! facts are exactly the `posting` table. On the cash basis they are the
//! postings with two balanced adjustments (see [`crate::cash`]):
//!
//! 1. an unpaid invoice's or bill's income and expense postings are moved into
//!    its A/R or A/P account (done in SQL, by re-pointing the posting's account,
//!    so it still aggregates in the database);
//! 2. each payment applied to one moves its share back out of A/R or A/P and into
//!    those income and expense accounts, on the payment's date (computed in Rust,
//!    exact to the cent).
//!
//! Each adjustment sums to zero on its own date, so a cash-basis balance sheet
//! balances as surely as an accrual one.
//!
//! Summary reports call [`Ledger::aggregate`], which groups in SQL by account,
//! period bucket and whatever dimension the columns need; only detail reports
//! call [`Ledger::facts`], and only for the rows they print.

use std::cell::OnceCell;
use std::collections::HashMap;

use jiff::civil::Date;
use lb_core::{
    AccountId, ClassId, ClearedStatus, DateRange, EntityId, ItemId, LineId, Money, TxnId, TxnType,
};
use lb_store::codec::{self, SqlDate};
use rusqlite::Connection;
use rust_decimal::Decimal;

use crate::ctx::Ctx;
use crate::filter::{Filter, Sql};
use crate::request::Basis;
use crate::{Result, cash};

/// One line of the ledger as a report sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    /// The transaction to open on QuickZoom. On cash basis, for income recognised
    /// from a payment, that is the payment.
    pub txn_id: TxnId,
    pub txn_type: TxnType,
    /// Entry order of the transaction (its rowid), to order a day's lines.
    pub txn_order: i64,
    pub seq: i64,
    pub date: Date,
    pub account_id: AccountId,
    /// Debit-positive.
    pub amount: Money,
    /// The posting's own name (a line's customer:job, an A/R line's customer).
    pub entity_id: Option<EntityId>,
    /// The transaction's name (payee, customer, vendor). On cash basis, for
    /// recognised income, the invoice's or bill's.
    pub txn_entity_id: Option<EntityId>,
    pub class_id: Option<ClassId>,
    pub item_id: Option<ItemId>,
    pub memo: Option<String>,
    pub cleared: ClearedStatus,
    pub number: Option<String>,
    pub quantity: Option<Decimal>,
    pub line_id: Option<LineId>,
    pub is_void: bool,
    /// The transaction's total (for the amount filter and detail reports).
    pub txn_amount: Money,
    /// Cash basis: the invoice or bill this amount was recognised from.
    pub origin: Option<TxnId>,
    /// The type of the transaction the amount comes from (the invoice's type for
    /// recognised income, else `txn_type`): sales and purchase reports select on
    /// this.
    pub origin_type: TxnType,
    /// Cash basis: an A/R or A/P amount standing in for deferred income or expense.
    pub repointed: bool,
}

/// How finely [`Ledger::aggregate`] splits dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gran {
    /// One bucket for the whole range.
    None,
    /// `YYYY-MM`.
    Month,
    /// `YYYY-MM-DD`.
    Day,
}

impl Gran {
    fn sql(self) -> &'static str {
        match self {
            Gran::None => "''",
            Gran::Month => "substr(f.date, 1, 7)",
            Gran::Day => "f.date",
        }
    }

    /// The bucket key of a date.
    pub fn key(self, d: Date) -> String {
        match self {
            Gran::None => String::new(),
            Gran::Month => format!("{:04}-{:02}", d.year(), d.month()),
            Gran::Day => d.to_string(),
        }
    }

    /// The dates a bucket key covers.
    pub fn span(self, key: &str) -> Option<(Date, Date)> {
        match self {
            Gran::None => None,
            Gran::Month => {
                let d: Date = format!("{key}-01").parse().ok()?;
                Some((d, d.last_of_month()))
            }
            Gran::Day => {
                let d: Date = key.parse().ok()?;
                Some((d, d))
            }
        }
    }
}

/// Which dimensions [`Ledger::aggregate`] groups by besides account and date.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dims {
    /// The posting's name and the transaction's name.
    pub names: bool,
    pub class: bool,
    pub item: bool,
    /// The type of the transaction the amount comes from.
    pub origin_type: bool,
}

/// One group's key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AggKey {
    pub account_id: AccountId,
    pub bucket: String,
    pub entity_id: Option<EntityId>,
    pub txn_entity_id: Option<EntityId>,
    pub class_id: Option<ClassId>,
    pub item_id: Option<ItemId>,
    pub origin_type: Option<TxnType>,
}

/// The ledger on one basis, through one filter.
#[derive(Debug)]
pub struct Ledger<'a> {
    pub c: &'a Connection,
    pub ctx: &'a Ctx,
    pub basis: Basis,
    pub filter: Filter,
    recognitions: OnceCell<Vec<Fact>>,
}

impl<'a> Ledger<'a> {
    pub fn new(c: &'a Connection, ctx: &'a Ctx, basis: Basis, filter: Filter) -> Ledger<'a> {
        Ledger {
            c,
            ctx,
            basis,
            filter,
            recognitions: OnceCell::new(),
        }
    }

    fn tax_on_cash(&self) -> bool {
        self.ctx.info.preferences.sales_tax_on_cash_basis
    }

    /// The SELECT producing one row per fact (alias it as `f`).
    fn base_sql(&self) -> String {
        let (acct, ent, item, qty, line, def) = match self.basis {
            Basis::Accrual => (
                "p.account_id".to_owned(),
                "p.entity_id".to_owned(),
                "p.item_id".to_owned(),
                "p.quantity".to_owned(),
                "p.line_id".to_owned(),
                "0".to_owned(),
            ),
            Basis::Cash => {
                let d = cash::deferred_sql(self.tax_on_cash());
                (
                    format!("CASE WHEN {d} THEN t.ar_ap_account_id ELSE p.account_id END"),
                    format!("CASE WHEN {d} THEN t.entity_id ELSE p.entity_id END"),
                    format!("CASE WHEN {d} THEN NULL ELSE p.item_id END"),
                    format!("CASE WHEN {d} THEN NULL ELSE p.quantity END"),
                    format!("CASE WHEN {d} THEN NULL ELSE p.line_id END"),
                    format!("CASE WHEN {d} THEN 1 ELSE 0 END"),
                )
            }
        };
        format!(
            "SELECT p.txn_id AS txn_id, p.txn_type AS txn_type, t.rowid AS txn_order, p.seq AS seq,
                    p.date AS date, {acct} AS account_id, p.amount AS amount, {ent} AS entity_id,
                    t.entity_id AS txn_entity_id, p.class_id AS class_id, {item} AS item_id,
                    COALESCE(p.memo, t.memo) AS memo, p.cleared AS cleared, t.number AS number,
                    {qty} AS quantity, {line} AS line_id, t.is_void AS is_void,
                    t.amount AS txn_amount, {def} AS repointed
             FROM posting p JOIN txn t ON t.id = p.txn_id JOIN account a ON a.id = p.account_id
                  LEFT JOIN item i ON i.id = p.item_id"
        )
    }

    fn range_sql(q: &mut Sql, range: DateRange) -> String {
        let mut out = String::new();
        if let Some(f) = range.from {
            let p = q.bind(f.to_string());
            out.push_str(&format!(" AND f.date >= {p}"));
        }
        if let Some(t) = range.to {
            let p = q.bind(t.to_string());
            out.push_str(&format!(" AND f.date <= {p}"));
        }
        out
    }

    /// Cash-basis income and expense recognised from payments (empty on accrual).
    fn recognitions(&self) -> Result<&[Fact]> {
        if self.basis == Basis::Accrual {
            return Ok(&[]);
        }
        if self.recognitions.get().is_none() {
            let r = cash::recognitions(self.c, self.ctx, self.tax_on_cash())?;
            let _ = self.recognitions.set(r);
        }
        Ok(self.recognitions.get().map_or(&[], Vec::as_slice))
    }

    /// Sums, debit-positive, grouped by account, date bucket and `dims`, for
    /// facts within `range` passing `filter`.
    pub fn aggregate_with(
        &self,
        filter: &Filter,
        range: DateRange,
        gran: Gran,
        dims: Dims,
    ) -> Result<HashMap<AggKey, Money>> {
        let mut q = Sql::default();
        let base = self.base_sql();
        let cond = format!("{}{}", Self::range_sql(&mut q, range), filter.sql(&mut q));
        let col = |on: bool, c: &str| if on { c.to_owned() } else { "NULL".to_owned() };
        q.text = format!(
            "SELECT f.account_id, {bucket}, {e}, {te}, {cl}, {it}, {ot}, SUM(f.amount)
             FROM ({base}) f WHERE 1 = 1{cond}
             GROUP BY 1, 2, 3, 4, 5, 6, 7",
            bucket = gran.sql(),
            e = col(dims.names, "f.entity_id"),
            te = col(dims.names, "f.txn_entity_id"),
            cl = col(dims.class, "f.class_id"),
            it = col(dims.item, "f.item_id"),
            ot = col(dims.origin_type, "f.txn_type"),
        );
        let mut out: HashMap<AggKey, Money> = HashMap::new();
        {
            let mut stmt = self.c.prepare(&q.text)?;
            let mut rows = stmt.query(q.params())?;
            while let Some(r) = rows.next()? {
                let key = AggKey {
                    account_id: codec::parse_id(&r.get::<_, String>(0)?)?,
                    bucket: r.get(1)?,
                    entity_id: opt_id(r.get(2)?)?,
                    txn_entity_id: opt_id(r.get(3)?)?,
                    class_id: opt_id(r.get(4)?)?,
                    item_id: opt_id(r.get(5)?)?,
                    origin_type: r
                        .get::<_, Option<String>>(6)?
                        .and_then(|s| TxnType::parse(&s)),
                };
                *out.entry(key).or_default() += Money(r.get(7)?);
            }
        }
        for f in self.recognitions()? {
            if !range.contains(f.date) || !filter.matches(f) {
                continue;
            }
            let key = AggKey {
                account_id: f.account_id,
                bucket: gran.key(f.date),
                entity_id: if dims.names { f.entity_id } else { None },
                txn_entity_id: if dims.names { f.txn_entity_id } else { None },
                class_id: if dims.class { f.class_id } else { None },
                item_id: if dims.item { f.item_id } else { None },
                origin_type: dims.origin_type.then_some(f.origin_type),
            };
            *out.entry(key).or_default() += f.amount;
        }
        Ok(out)
    }

    /// [`Ledger::aggregate_with`] through this ledger's own filter.
    pub fn aggregate(
        &self,
        range: DateRange,
        gran: Gran,
        dims: Dims,
    ) -> Result<HashMap<AggKey, Money>> {
        self.aggregate_with(&self.filter, range, gran, dims)
    }

    /// Debit-positive balance per account over `range` (through the filter).
    pub fn sums(&self, range: DateRange) -> Result<HashMap<AccountId, Money>> {
        let mut out: HashMap<AccountId, Money> = HashMap::new();
        for (k, v) in self.aggregate(range, Gran::None, Dims::default())? {
            *out.entry(k.account_id).or_default() += v;
        }
        Ok(out)
    }

    /// The facts within `range` passing `filter`, in date and entry order. On cash
    /// basis, a transaction's A/R or A/P amounts in one account are merged into one
    /// line (they are bookkeeping for the basis, not lines anyone entered).
    pub fn facts_with(&self, filter: &Filter, range: DateRange) -> Result<Vec<Fact>> {
        let mut q = Sql::default();
        let base = self.base_sql();
        let cond = format!("{}{}", Self::range_sql(&mut q, range), filter.sql(&mut q));
        q.text = format!(
            "SELECT f.txn_id, f.txn_type, f.txn_order, f.seq, f.date, f.account_id, f.amount,
                    f.entity_id, f.txn_entity_id, f.class_id, f.item_id, f.memo, f.cleared,
                    f.number, f.quantity, f.line_id, f.is_void, f.txn_amount, f.repointed
             FROM ({base}) f WHERE 1 = 1{cond}
             ORDER BY f.date, f.txn_order, f.seq"
        );
        let mut out = Vec::new();
        {
            let mut stmt = self.c.prepare(&q.text)?;
            let mut rows = stmt.query(q.params())?;
            while let Some(r) = rows.next()? {
                let txn_type: String = r.get(1)?;
                let txn_type = TxnType::parse(&txn_type)
                    .ok_or_else(|| lb_store::StoreError::Corrupt(txn_type.clone()))?;
                let date: SqlDate = r.get(4)?;
                out.push(Fact {
                    txn_id: codec::parse_id(&r.get::<_, String>(0)?)?,
                    txn_type,
                    txn_order: r.get(2)?,
                    seq: r.get(3)?,
                    date: date.0,
                    account_id: codec::parse_id(&r.get::<_, String>(5)?)?,
                    amount: Money(r.get(6)?),
                    entity_id: opt_id(r.get(7)?)?,
                    txn_entity_id: opt_id(r.get(8)?)?,
                    class_id: opt_id(r.get(9)?)?,
                    item_id: opt_id(r.get(10)?)?,
                    memo: r.get(11)?,
                    cleared: lb_store::txns::parse_cleared(&r.get::<_, String>(12)?),
                    number: r.get(13)?,
                    quantity: r.get::<_, Option<String>>(14)?.and_then(|q| q.parse().ok()),
                    line_id: opt_id(r.get(15)?)?,
                    is_void: r.get(16)?,
                    txn_amount: Money(r.get(17)?),
                    origin: None,
                    origin_type: txn_type,
                    repointed: r.get(18)?,
                });
            }
        }
        if self.basis == Basis::Cash {
            out.extend(
                self.recognitions()?
                    .iter()
                    .filter(|f| range.contains(f.date) && filter.matches(f))
                    .cloned(),
            );
            out.sort_by(|a, b| {
                (a.date, a.txn_order, a.seq, a.account_id).cmp(&(
                    b.date,
                    b.txn_order,
                    b.seq,
                    b.account_id,
                ))
            });
            out = merge_repointed(out);
        }
        Ok(out)
    }

    /// [`Ledger::facts_with`] through this ledger's own filter.
    pub fn facts(&self, range: DateRange) -> Result<Vec<Fact>> {
        self.facts_with(&self.filter, range)
    }

    /// Quantities sold per item: the form lines behind the facts passing `filter`
    /// within `range` whose transaction type is one of `types` (quantities live on
    /// the form, not on income postings). Credit memos count negative. Summed
    /// exactly as decimals, grouped in SQL by the quantity's text first. On cash
    /// basis this counts only sales not held open in A/R (sales receipts and the
    /// like): a payment recognises an amount, not a quantity.
    pub fn item_quantities(
        &self,
        filter: &Filter,
        range: DateRange,
        types: &[TxnType],
    ) -> Result<HashMap<Option<ItemId>, Decimal>> {
        let mut q = Sql::default();
        let base = self.base_sql();
        let cond = format!("{}{}", Self::range_sql(&mut q, range), filter.sql(&mut q));
        let list = q.bind_list(types.iter().map(|t| t.as_str().to_owned()));
        q.text = format!(
            "SELECT f.item_id, f.txn_type, json_extract(j.value, '$.quantity'), COUNT(*)
             FROM ({base}) f JOIN txn t2 ON t2.id = f.txn_id, json_each(t2.doc, '$.lines') j
             WHERE f.line_id IS NOT NULL AND f.txn_type IN {list}
               AND json_extract(j.value, '$.id') = f.line_id{cond}
             GROUP BY 1, 2, 3"
        );
        let mut out: HashMap<Option<ItemId>, Decimal> = HashMap::new();
        let mut stmt = self.c.prepare(&q.text)?;
        let mut rows = stmt.query(q.params())?;
        while let Some(r) = rows.next()? {
            let item: Option<ItemId> = opt_id(r.get(0)?)?;
            let t: String = r.get(1)?;
            let qty: Option<String> = r.get(2)?;
            let n: i64 = r.get(3)?;
            let Some(qty) = qty.and_then(|s| s.parse::<Decimal>().ok()) else {
                continue;
            };
            let sign = if t == TxnType::CreditMemo.as_str() {
                -Decimal::ONE
            } else {
                Decimal::ONE
            };
            *out.entry(item).or_default() += qty * Decimal::from(n) * sign;
        }
        Ok(out)
    }

    /// The date of the earliest fact, for "All dates" period columns.
    pub fn first_date(&self) -> Result<Option<Date>> {
        let d: Option<SqlDate> = self
            .c
            .query_row("SELECT MIN(date) FROM posting", [], |r| r.get(0))?;
        Ok(d.map(|d| d.0))
    }
}

/// Fold each transaction's re-pointed A/R or A/P amounts into one line per
/// account (into the transaction's own line there, when it has one), dropping
/// lines that come to nothing.
fn merge_repointed(facts: Vec<Fact>) -> Vec<Fact> {
    let mut out: Vec<Fact> = Vec::with_capacity(facts.len());
    // (txn, account) → index in `out` of the line absorbing re-pointed amounts.
    let mut home: HashMap<(TxnId, AccountId), usize> = HashMap::new();
    let mut absorbed: Vec<bool> = Vec::new();
    for f in facts {
        let key = (f.txn_id, f.account_id);
        if let Some(&i) = home.get(&key)
            && (f.repointed || out[i].repointed)
        {
            out[i].amount += f.amount;
            if !f.repointed {
                // Keep the real line's details.
                let amount = out[i].amount;
                out[i] = Fact { amount, ..f };
            }
            absorbed[i] = true;
            continue;
        }
        home.insert(key, out.len());
        out.push(f);
        absorbed.push(false);
    }
    out.into_iter()
        .zip(absorbed)
        .filter(|(f, merged)| !(*merged || f.repointed) || !f.amount.is_zero())
        .map(|(f, _)| f)
        .collect()
}

fn opt_id<T: std::str::FromStr>(s: Option<String>) -> Result<Option<T>> {
    Ok(s.map(|s| codec::parse_id(&s)).transpose()?)
}
