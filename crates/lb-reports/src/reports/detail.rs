//! Detail reports: the lines behind the summaries.
//!
//! General Ledger, Transaction Detail by Account, Account QuickReport, Balance
//! Sheet Detail and Profit & Loss Detail group lines by account (indented under
//! parent accounts as the summaries are, each with its total and running
//! balance); Journal groups them by transaction; Transaction List by Date gives
//! one line per transaction. Every line opens its transaction.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use lb_core::{AccountId, AccountRole, AccountType, ClearedStatus, DateRange, Money, TxnId};
use lb_store::codec;

use super::{PL_SECTIONS, accounts_of};
use crate::dates::fiscal_year_start;
use crate::filter::Sql;
use crate::ledger::Fact;
use crate::model::{Cell, Column, ColumnKind, Report, Row, RowKind};
use crate::request::SortBy;
use crate::{ReportError, Result, Run};

/// The columns detail reports draw from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DCol {
    TransNo,
    Type,
    Date,
    Num,
    Name,
    Memo,
    Account,
    Split,
    Clr,
    Class,
    Item,
    Debit,
    Credit,
    Amount,
    Balance,
}

impl DCol {
    pub fn column(self) -> Column {
        let (id, label, kind) = match self {
            DCol::TransNo => ("trans", "Trans #", ColumnKind::Number),
            DCol::Type => ("type", "Type", ColumnKind::Text),
            DCol::Date => ("date", "Date", ColumnKind::Date),
            DCol::Num => ("num", "Num", ColumnKind::Text),
            DCol::Name => ("name", "Name", ColumnKind::Text),
            DCol::Memo => ("memo", "Memo", ColumnKind::Text),
            DCol::Account => ("account", "Account", ColumnKind::Text),
            DCol::Split => ("split", "Split", ColumnKind::Text),
            DCol::Clr => ("clr", "Clr", ColumnKind::Text),
            DCol::Class => ("class", "Class", ColumnKind::Text),
            DCol::Item => ("item", "Item", ColumnKind::Text),
            DCol::Debit => ("debit", "Debit", ColumnKind::Money),
            DCol::Credit => ("credit", "Credit", ColumnKind::Money),
            DCol::Amount => ("amount", "Amount", ColumnKind::Money),
            DCol::Balance => ("balance", "Balance", ColumnKind::Money),
        };
        Column::new(id, label, kind)
    }
}

/// The label column plus `cols`.
pub(crate) fn columns(cols: &[DCol]) -> Vec<Column> {
    let mut out = vec![Column::new("label", "", ColumnKind::Text)];
    out.extend(cols.iter().map(|c| c.column()));
    out
}

pub(crate) fn cleared_mark(c: ClearedStatus) -> &'static str {
    match c {
        ClearedStatus::Cleared => "√",
        ClearedStatus::Pending => "*",
        ClearedStatus::Uncleared => "",
    }
}

/// Lazily loaded "other side" accounts for the Split column.
pub(crate) struct Splits {
    accounts: HashMap<TxnId, Vec<AccountId>>,
}

impl Splits {
    /// Load the accounts of every transaction the facts belong to.
    pub fn load(r: &Run<'_>, facts: &[Fact]) -> Result<Splits> {
        let ids: Vec<TxnId> = facts
            .iter()
            .map(|f| f.txn_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut accounts: HashMap<TxnId, Vec<AccountId>> = HashMap::new();
        for chunk in ids.chunks(500) {
            let mut q = Sql::default();
            let list = q.bind_list(chunk.iter().map(ToString::to_string));
            q.text =
                format!("SELECT DISTINCT txn_id, account_id FROM posting WHERE txn_id IN {list}");
            let mut stmt = r.c.prepare(&q.text)?;
            let mut rows = stmt.query(q.params())?;
            while let Some(row) = rows.next()? {
                let t: TxnId = codec::parse_id(&row.get::<_, String>(0)?)?;
                let a: AccountId = codec::parse_id(&row.get::<_, String>(1)?)?;
                accounts.entry(t).or_default().push(a);
            }
        }
        Ok(Splits { accounts })
    }

    /// The account on the other side of a line, or "-SPLIT-" when several.
    pub fn label(&self, r: &Run<'_>, f: &Fact) -> String {
        let others: Vec<AccountId> = self
            .accounts
            .get(&f.txn_id)
            .map(|v| v.iter().copied().filter(|a| *a != f.account_id).collect())
            .unwrap_or_default();
        match others.as_slice() {
            [] => String::new(),
            [one] => r.ctx.account_full_label(*one),
            _ => "-SPLIT-".to_owned(),
        }
    }
}

/// One fact's cells for `cols`. `amount` is the line's amount as shown;
/// `balance` the running balance after it.
pub(crate) fn fact_cells(
    r: &Run<'_>,
    splits: &Splits,
    f: &Fact,
    cols: &[DCol],
    amount: Money,
    balance: Option<Money>,
    trans_no: Option<usize>,
) -> Vec<Cell> {
    let ctx = r.ctx;
    let mut cells = vec![Cell::EMPTY];
    for c in cols {
        cells.push(match c {
            DCol::TransNo => trans_no.map_or(Cell::EMPTY, |n| Cell::number(n.to_string())),
            DCol::Type => Cell::text(f.txn_type.label()),
            DCol::Date => Cell::date(f.date),
            DCol::Num => Cell::opt_text(f.number.clone()),
            DCol::Name => Cell::opt_text(ctx.entity_name(f.entity_id.or(f.txn_entity_id))),
            DCol::Memo => Cell::opt_text(f.memo.clone()),
            DCol::Account => Cell::text(ctx.account_full_label(f.account_id)),
            DCol::Split => Cell::text(splits.label(r, f)),
            DCol::Clr => Cell::text(cleared_mark(f.cleared)),
            DCol::Class => Cell::opt_text(ctx.class_name(f.class_id)),
            DCol::Item => Cell::opt_text(
                f.item_id
                    .and_then(|i| ctx.item(i))
                    .map(|i| i.full_name.clone()),
            ),
            DCol::Debit => {
                if f.amount.is_positive() {
                    Cell::money(f.amount)
                } else {
                    Cell::EMPTY
                }
            }
            DCol::Credit => {
                if f.amount.is_negative() {
                    Cell::money(-f.amount)
                } else {
                    Cell::EMPTY
                }
            }
            DCol::Amount => Cell::money(amount),
            DCol::Balance => balance.map_or(Cell::EMPTY, Cell::money),
        });
    }
    cells
}

/// The amounts a total row shows in the money columns.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Totals {
    pub debit: Money,
    pub credit: Money,
    pub amount: Money,
    pub balance: Option<Money>,
}

/// A row of totals: `label` first, then amounts in the money columns.
pub(crate) fn total_row(kind: RowKind, depth: u32, label: String, cols: &[DCol], t: Totals) -> Row {
    let Totals {
        debit,
        credit,
        amount,
        balance,
    } = t;
    let mut cells = vec![Cell::text(label)];
    for c in cols {
        cells.push(match c {
            DCol::Debit => Cell::money(debit),
            DCol::Credit => Cell::money(credit),
            DCol::Amount => Cell::money(amount),
            DCol::Balance => balance.map_or(Cell::EMPTY, Cell::money),
            _ => Cell::EMPTY,
        });
    }
    Row::new(kind, depth, cells)
}

/// Order facts within a group as the request asks (stable).
pub(crate) fn sort_facts(r: &Run<'_>, facts: &mut [Fact]) {
    let ctx = r.ctx;
    let by = r.req.sort_by;
    if by == SortBy::Default {
        if r.req.sort_descending {
            facts.reverse();
        }
        return;
    }
    let key = |f: &Fact, g: &Fact| -> Ordering {
        match by {
            SortBy::Default | SortBy::Date => f.date.cmp(&g.date),
            SortBy::Number => natural_cmp(f.number.as_deref(), g.number.as_deref()),
            SortBy::Name => ctx
                .entity_name(f.entity_id.or(f.txn_entity_id))
                .unwrap_or_default()
                .to_lowercase()
                .cmp(
                    &ctx.entity_name(g.entity_id.or(g.txn_entity_id))
                        .unwrap_or_default()
                        .to_lowercase(),
                ),
            SortBy::Memo => f
                .memo
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&g.memo.as_deref().unwrap_or("").to_lowercase()),
            SortBy::Amount => f.amount.abs().cmp(&g.amount.abs()),
        }
    };
    facts.sort_by(|a, b| {
        let o = key(a, b);
        if r.req.sort_descending {
            o.reverse()
        } else {
            o
        }
    });
}

/// Numbers compare as numbers when both are ("9" before "10").
fn natural_cmp(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (
        a.and_then(|x| x.parse::<u64>().ok()),
        b.and_then(|x| x.parse::<u64>().ok()),
    ) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.unwrap_or("").cmp(b.unwrap_or("")),
    }
}

/// What account-grouped sections need.
pub(crate) struct AccountSections<'x> {
    pub cols: &'x [DCol],
    /// Each account's lines, in order.
    pub facts: HashMap<AccountId, Vec<Fact>>,
    /// Balance brought forward, debit-positive (empty: balances start at zero).
    pub opening: HashMap<AccountId, Money>,
    pub splits: &'x Splits,
    pub show_zero: bool,
}

/// Sums of a section: debits, credits, amount (as shown), ending balance.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sums {
    pub debit: Money,
    pub credit: Money,
    pub amount: Money,
    pub balance: Money,
}

impl std::ops::AddAssign for Sums {
    fn add_assign(&mut self, o: Sums) {
        self.debit += o.debit;
        self.credit += o.credit;
        self.amount += o.amount;
        self.balance += o.balance;
    }
}

/// Accounts in `members`, indented under their parents, each with its lines,
/// its running balance in its natural sign, and a total line.
pub(crate) fn account_sections(
    r: &Run<'_>,
    s: &mut AccountSections<'_>,
    members: &[AccountId],
    depth: u32,
) -> (Vec<Row>, Sums) {
    let ctx = r.ctx;
    let set: HashSet<AccountId> = members.iter().copied().collect();
    let mut roots: Vec<AccountId> = members
        .iter()
        .copied()
        .filter(|a| {
            ctx.account(*a)
                .and_then(|x| x.parent_id)
                .is_none_or(|p| !set.contains(&p))
        })
        .collect();
    roots.sort_by_key(|a| ctx.account_order(*a));
    let mut rows = Vec::new();
    let mut sums = Sums::default();
    for a in roots {
        if let Some((rs, sm)) = account_section(r, s, &set, a, depth) {
            rows.extend(rs);
            sums += sm;
        }
    }
    (rows, sums)
}

fn account_section(
    r: &Run<'_>,
    s: &mut AccountSections<'_>,
    set: &HashSet<AccountId>,
    id: AccountId,
    depth: u32,
) -> Option<(Vec<Row>, Sums)> {
    let ctx = r.ctx;
    let mut kids: Vec<AccountId> = ctx
        .account_children(id)
        .iter()
        .copied()
        .filter(|c| set.contains(c))
        .collect();
    kids.sort_by_key(|a| ctx.account_order(*a));
    let mut child_rows = Vec::new();
    let mut child_sums = Sums::default();
    for k in kids {
        if let Some((rs, sm)) = account_section(r, s, set, k, depth + 1) {
            child_rows.extend(rs);
            child_sums += sm;
        }
    }
    let label = ctx.account_label(id);
    let has_children = !child_rows.is_empty();
    let own_depth = if has_children { depth + 1 } else { depth };
    let own_label = if has_children {
        format!("{label} - Other")
    } else {
        label.clone()
    };
    let own = own_section(r, s, id, &own_label, own_depth, has_children);
    if !has_children && own.is_none() {
        return None;
    }
    let mut rows = Vec::new();
    let mut sums = child_sums;
    if has_children {
        rows.push(
            Row::new(
                RowKind::Header,
                depth,
                header_cells(s.cols, label.clone(), None),
            )
            .drill(r.drill_to(
                crate::request::ReportId::AccountQuickReport,
                r.range,
                |f| {
                    f.accounts = vec![id];
                    f.exact = false;
                },
            )),
        );
        rows.extend(child_rows);
    }
    if let Some((rs, sm)) = own {
        rows.extend(rs);
        sums += sm;
    }
    if has_children {
        rows.push(total_row(
            RowKind::Subtotal,
            depth,
            format!("Total {label}"),
            s.cols,
            Totals {
                debit: sums.debit,
                credit: sums.credit,
                amount: sums.amount,
                balance: s.cols.contains(&DCol::Balance).then_some(sums.balance),
            },
        ));
    }
    Some((rows, sums))
}

fn header_cells(cols: &[DCol], label: String, balance: Option<Money>) -> Vec<Cell> {
    let mut cells = vec![Cell::text(label)];
    for c in cols {
        cells.push(match (c, balance) {
            (DCol::Balance, Some(b)) => Cell::money(b),
            _ => Cell::EMPTY,
        });
    }
    cells
}

/// An account's own lines: heading (with the balance brought forward), lines,
/// total. `None` when it has nothing to show.
fn own_section(
    r: &Run<'_>,
    s: &mut AccountSections<'_>,
    id: AccountId,
    label: &str,
    depth: u32,
    is_other: bool,
) -> Option<(Vec<Row>, Sums)> {
    let ctx = r.ctx;
    let t = ctx.account_type(id).unwrap_or(AccountType::Bank);
    let mut facts = s.facts.remove(&id).unwrap_or_default();
    let opening_dp = s.opening.get(&id).copied().unwrap_or_default();
    let active = ctx.account(id).is_some_and(|a| a.is_active);
    if facts.is_empty() && opening_dp.is_zero() && !(s.show_zero && active && !is_other) {
        return None;
    }
    sort_facts(r, &mut facts);
    let with_balance = s.cols.contains(&DCol::Balance);
    let opening = t.natural(opening_dp);
    let mut rows = vec![
        Row::new(
            RowKind::Header,
            depth,
            header_cells(s.cols, label.to_owned(), with_balance.then_some(opening)),
        )
        .drill(
            r.drill_to(crate::request::ReportId::AccountQuickReport, r.range, |f| {
                f.accounts = vec![id];
                f.exact = true;
            }),
        ),
    ];
    let mut sums = Sums {
        balance: opening,
        ..Sums::default()
    };
    for f in &facts {
        let amt = t.natural(f.amount);
        sums.amount += amt;
        sums.balance += amt;
        if f.amount.is_positive() {
            sums.debit += f.amount;
        } else {
            sums.credit += -f.amount;
        }
        rows.push(
            Row::new(
                RowKind::Data,
                depth + 1,
                fact_cells(
                    r,
                    s.splits,
                    f,
                    s.cols,
                    amt,
                    with_balance.then_some(sums.balance),
                    None,
                ),
            )
            .drill(Run::txn_drill(f.txn_id)),
        );
    }
    rows.push(total_row(
        RowKind::Subtotal,
        depth,
        format!("Total {label}"),
        s.cols,
        Totals {
            debit: sums.debit,
            credit: sums.credit,
            amount: sums.amount,
            balance: with_balance.then_some(sums.balance),
        },
    ));
    Some((rows, sums))
}

/// Group facts by account.
fn by_account_map(facts: Vec<Fact>) -> HashMap<AccountId, Vec<Fact>> {
    let mut m: HashMap<AccountId, Vec<Fact>> = HashMap::new();
    for f in facts {
        m.entry(f.account_id).or_default().push(f);
    }
    m
}

/// Balances brought forward to `from`: balance-sheet accounts from the
/// beginning, income and expense accounts from the start of `from`'s fiscal year,
/// and Retained Earnings carrying earlier years' net income. Empty when the range
/// is open at the start.
fn opening_balances(
    r: &Run<'_>,
    from: Option<jiff::civil::Date>,
) -> Result<HashMap<AccountId, Money>> {
    let Some(from) = from else {
        return Ok(HashMap::new());
    };
    let ctx = r.ctx;
    let Ok(before) = from.yesterday() else {
        return Ok(HashMap::new());
    };
    let ledger = r.ledger();
    let all = ledger.sums(DateRange {
        from: None,
        to: Some(before),
    })?;
    let fys = fiscal_year_start(from, ctx.fiscal_start());
    let this_year = if fys <= before {
        ledger.sums(DateRange::new(fys, before))?
    } else {
        HashMap::new()
    };
    let mut out = HashMap::new();
    let mut prior_pl = Money::ZERO;
    for (a, v) in &all {
        if ctx
            .account_type(*a)
            .is_some_and(AccountType::is_income_statement)
        {
            let cur = this_year.get(a).copied().unwrap_or_default();
            prior_pl += *v - cur;
            out.insert(*a, cur);
        } else {
            out.insert(*a, *v);
        }
    }
    if let Some(re) = ctx.role(AccountRole::RetainedEarnings) {
        *out.entry(re).or_default() += prior_pl;
    }
    Ok(out)
}

pub(crate) fn general_ledger(r: &Run<'_>) -> Result<Report> {
    let cols = [
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Name,
        DCol::Memo,
        DCol::Split,
        DCol::Debit,
        DCol::Credit,
        DCol::Balance,
    ];
    let facts = r.ledger().facts(r.range)?;
    let splits = Splits::load(r, &facts)?;
    let opening = opening_balances(r, r.range.from)?;
    let members: Vec<AccountId> = r
        .ctx
        .accounts
        .iter()
        .filter(|a| a.account_type != AccountType::NonPosting)
        .filter(|a| r.filter.accounts.as_ref().is_none_or(|s| s.contains(&a.id)))
        .map(|a| a.id)
        .collect();
    let mut s = AccountSections {
        cols: &cols,
        facts: by_account_map(facts),
        opening,
        splits: &splits,
        show_zero: r.req.show_zero_rows,
    };
    let (mut rows, sums) = account_sections(r, &mut s, &members, 0);
    rows.push(total_row(
        RowKind::Total,
        0,
        "TOTAL".into(),
        &cols,
        Totals {
            debit: sums.debit,
            credit: sums.credit,
            amount: sums.amount,
            balance: None,
        },
    ));
    Ok(r.finish(r.range_line(), columns(&cols), rows, Vec::new()))
}

pub(crate) fn by_account(r: &Run<'_>) -> Result<Report> {
    let cols = [
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Name,
        DCol::Memo,
        DCol::Clr,
        DCol::Split,
        DCol::Amount,
        DCol::Balance,
    ];
    let facts = r.ledger().facts(r.range)?;
    let splits = Splits::load(r, &facts)?;
    let members: Vec<AccountId> = r.ctx.accounts.iter().map(|a| a.id).collect();
    let mut s = AccountSections {
        cols: &cols,
        facts: by_account_map(facts),
        opening: HashMap::new(),
        splits: &splits,
        show_zero: false,
    };
    let (mut rows, sums) = account_sections(r, &mut s, &members, 0);
    rows.push(total_row(
        RowKind::Total,
        0,
        "TOTAL".into(),
        &cols,
        Totals {
            debit: sums.debit,
            credit: sums.credit,
            amount: sums.amount,
            balance: Some(sums.balance),
        },
    ));
    Ok(r.finish(r.range_line(), columns(&cols), rows, Vec::new()))
}

pub(crate) fn quick_report(r: &Run<'_>) -> Result<Report> {
    if r.req.filters.accounts.is_empty() {
        return Err(ReportError::Invalid(
            "an Account QuickReport needs an account (filters.accounts)".into(),
        ));
    }
    let cols = [
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Name,
        DCol::Memo,
        DCol::Split,
        DCol::Amount,
        DCol::Balance,
    ];
    let facts = r.ledger().facts(r.range)?;
    let splits = Splits::load(r, &facts)?;
    let members: Vec<AccountId> = r
        .filter
        .accounts
        .as_ref()
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    let opening = opening_balances(r, r.range.from)?;
    let mut s = AccountSections {
        cols: &cols,
        facts: by_account_map(facts),
        opening,
        splits: &splits,
        show_zero: true,
    };
    let (mut rows, sums) = account_sections(r, &mut s, &members, 0);
    rows.push(total_row(
        RowKind::Total,
        0,
        "TOTAL".into(),
        &cols,
        Totals {
            debit: sums.debit,
            credit: sums.credit,
            amount: sums.amount,
            balance: Some(sums.balance),
        },
    ));
    let mut report = r.finish(r.range_line(), columns(&cols), rows, Vec::new());
    if r.req.title.is_none()
        && let [one] = r.req.filters.accounts.as_slice()
    {
        report.title = r.ctx.account_full_label(*one);
    }
    Ok(report)
}

pub(crate) fn balance_sheet_detail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let cols = [
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Name,
        DCol::Memo,
        DCol::Split,
        DCol::Amount,
        DCol::Balance,
    ];
    let to = r.range.to.unwrap_or(ctx.today);
    let fs = ctx.fiscal_start();
    let mut notes = Vec::new();
    let ledger = r.ledger();
    let bs_types: Vec<AccountType> = AccountType::ALL
        .into_iter()
        .filter(|t| t.is_balance_sheet())
        .collect();
    let bs: HashSet<AccountId> = accounts_of(ctx, &bs_types).into_iter().collect();
    let facts: Vec<Fact> = ledger
        .facts(DateRange {
            from: r.range.from,
            to: Some(to),
        })?
        .into_iter()
        .filter(|f| bs.contains(&f.account_id))
        .collect();
    let splits = Splits::load(r, &facts)?;
    let mut opening = opening_balances(r, r.range.from)?;
    opening.retain(|a, _| bs.contains(a));
    // Retained earnings as of `to`: earlier fiscal years' net income.
    let fys_to = fiscal_year_start(to, fs);
    let ni_range = DateRange::new(fys_to, to);
    let pl = |m: &HashMap<AccountId, Money>| -> Money {
        m.iter()
            .filter(|(a, _)| {
                ctx.account_type(**a)
                    .is_some_and(AccountType::is_income_statement)
            })
            .map(|(_, v)| *v)
            .sum()
    };
    let net_income_dp = pl(&ledger.sums(ni_range)?);
    if let (Some(from), Some(re)) = (r.range.from, ctx.role(AccountRole::RetainedEarnings))
        && fiscal_year_start(from, fs) < fys_to
    {
        // The range crosses a fiscal year end: close the earlier year(s) into the
        // retained earnings brought forward, so the ending balance is as of `to`.
        let closed = pl(&ledger.sums(DateRange::new(
            fiscal_year_start(from, fs),
            fys_to.yesterday().unwrap_or(fys_to),
        ))?);
        *opening.entry(re).or_default() += closed;
        notes.push(format!(
            "Retained Earnings includes net income for fiscal years before {}.",
            crate::dates::long_date(fys_to)
        ));
    }
    let mut s = AccountSections {
        cols: &cols,
        facts: by_account_map(facts),
        opening,
        splits: &splits,
        show_zero: r.req.show_zero_rows,
    };
    let assets = accounts_of(
        ctx,
        &[
            AccountType::Bank,
            AccountType::AccountsReceivable,
            AccountType::OtherCurrentAsset,
            AccountType::FixedAsset,
            AccountType::OtherAsset,
        ],
    );
    let liab_eq = accounts_of(
        ctx,
        &[
            AccountType::AccountsPayable,
            AccountType::CreditCard,
            AccountType::OtherCurrentLiability,
            AccountType::LongTermLiability,
            AccountType::Equity,
        ],
    );
    let mut rows = vec![Row::new(
        RowKind::Header,
        0,
        header_cells(&cols, "ASSETS".into(), None),
    )];
    let (a_rows, a_sums) = account_sections(r, &mut s, &assets, 1);
    rows.extend(a_rows);
    rows.push(total_row(
        RowKind::Total,
        0,
        "TOTAL ASSETS".into(),
        &cols,
        Totals {
            debit: a_sums.debit,
            credit: a_sums.credit,
            amount: a_sums.amount,
            balance: Some(a_sums.balance),
        },
    ));
    rows.push(Row::new(
        RowKind::Header,
        0,
        header_cells(&cols, "LIABILITIES & EQUITY".into(), None),
    ));
    let (l_rows, mut l_sums) = account_sections(r, &mut s, &liab_eq, 1);
    rows.extend(l_rows);
    let ni = -net_income_dp;
    let mut ni_row = total_row(
        RowKind::Data,
        1,
        "Net Income".into(),
        &cols,
        Totals {
            debit: Money::ZERO,
            credit: Money::ZERO,
            amount: ni,
            balance: Some(ni),
        },
    );
    ni_row.drill = r.drill_to(crate::request::ReportId::ProfitAndLoss, ni_range, |_| {});
    rows.push(ni_row);
    l_sums.amount += ni;
    l_sums.balance += ni;
    rows.push(total_row(
        RowKind::Total,
        0,
        "TOTAL LIABILITIES & EQUITY".into(),
        &cols,
        Totals {
            debit: l_sums.debit,
            credit: l_sums.credit,
            amount: l_sums.amount,
            balance: Some(l_sums.balance),
        },
    ));
    Ok(r.finish(
        crate::dates::range_line(DateRange {
            from: r.range.from,
            to: Some(to),
        }),
        columns(&cols),
        rows,
        notes,
    ))
}

pub(crate) fn pnl_detail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let cols = [
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Name,
        DCol::Memo,
        DCol::Class,
        DCol::Clr,
        DCol::Split,
        DCol::Amount,
        DCol::Balance,
    ];
    let pl_types: Vec<AccountType> = PL_SECTIONS.iter().map(|(_, t)| *t).collect();
    let pl: HashSet<AccountId> = accounts_of(ctx, &pl_types).into_iter().collect();
    let facts: Vec<Fact> = r
        .ledger()
        .facts(r.range)?
        .into_iter()
        .filter(|f| pl.contains(&f.account_id))
        .collect();
    let splits = Splits::load(r, &facts)?;
    let mut s = AccountSections {
        cols: &cols,
        facts: by_account_map(facts),
        opening: HashMap::new(),
        splits: &splits,
        show_zero: r.req.show_zero_rows,
    };
    let mut parts: HashMap<AccountType, (Vec<Row>, Money)> = HashMap::new();
    for (_, t) in PL_SECTIONS {
        let (rows, sums) = account_sections(r, &mut s, &accounts_of(ctx, &[t]), 2);
        parts.insert(t, (rows, sums.amount));
    }
    let amt = |t: AccountType| parts.get(&t).map_or(Money::ZERO, |p| p.1);
    let present = |t: AccountType| parts.get(&t).is_some_and(|p| !p.0.is_empty());
    let income = amt(AccountType::Income);
    let cogs = amt(AccountType::CostOfGoodsSold);
    let expense = amt(AccountType::Expense);
    let oi = amt(AccountType::OtherIncome);
    let oe = amt(AccountType::OtherExpense);
    let ordinary = present(AccountType::Income)
        || present(AccountType::CostOfGoodsSold)
        || present(AccountType::Expense);
    let other = present(AccountType::OtherIncome) || present(AccountType::OtherExpense);
    let has_cogs = present(AccountType::CostOfGoodsSold);
    let line = |kind, depth, label: &str, v: Money| {
        total_row(
            kind,
            depth,
            label.to_owned(),
            &cols,
            Totals {
                debit: Money::ZERO,
                credit: Money::ZERO,
                amount: v,
                balance: Some(v),
            },
        )
    };
    let head = |depth, label: &str| {
        Row::new(
            RowKind::Header,
            depth,
            header_cells(&cols, label.into(), None),
        )
    };
    let mut rows = Vec::new();
    let mut section = |rows: &mut Vec<Row>, label: &str, total: &str, t: AccountType| {
        if let Some((rs, v)) = parts.remove(&t)
            && !rs.is_empty()
        {
            rows.push(head(1, label));
            rows.extend(rs);
            rows.push(line(RowKind::Subtotal, 1, total, v));
        }
    };
    if ordinary {
        rows.push(head(0, "Ordinary Income/Expense"));
        section(&mut rows, "Income", "Total Income", AccountType::Income);
        section(
            &mut rows,
            "Cost of Goods Sold",
            "Total COGS",
            AccountType::CostOfGoodsSold,
        );
        if has_cogs {
            rows.push(line(RowKind::Subtotal, 1, "Gross Profit", income - cogs));
        }
        section(&mut rows, "Expense", "Total Expense", AccountType::Expense);
        rows.push(line(
            RowKind::Subtotal,
            0,
            "Net Ordinary Income",
            income - cogs - expense,
        ));
    }
    if other {
        rows.push(head(0, "Other Income/Expense"));
        section(
            &mut rows,
            "Other Income",
            "Total Other Income",
            AccountType::OtherIncome,
        );
        section(
            &mut rows,
            "Other Expense",
            "Total Other Expense",
            AccountType::OtherExpense,
        );
        rows.push(line(RowKind::Subtotal, 0, "Net Other Income", oi - oe));
    }
    rows.push(line(
        RowKind::Total,
        0,
        "Net Income",
        income - cogs - expense + oi - oe,
    ));
    Ok(r.finish(r.range_line(), columns(&cols), rows, Vec::new()))
}

pub(crate) fn journal(r: &Run<'_>) -> Result<Report> {
    let cols = [
        DCol::TransNo,
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Name,
        DCol::Memo,
        DCol::Account,
        DCol::Debit,
        DCol::Credit,
    ];
    let facts = r.ledger().facts(r.range)?;
    let splits = Splits::load(r, &facts)?;
    let mut groups: Vec<(TxnId, Vec<Fact>)> = Vec::new();
    let mut index: HashMap<TxnId, usize> = HashMap::new();
    for f in facts {
        if let Some(&i) = index.get(&f.txn_id) {
            groups[i].1.push(f);
        } else {
            index.insert(f.txn_id, groups.len());
            groups.push((f.txn_id, vec![f]));
        }
    }
    if r.req.sort_by != SortBy::Default || r.req.sort_descending {
        let mut firsts: Vec<Fact> = groups.iter().map(|g| g.1[0].clone()).collect();
        sort_facts(r, &mut firsts);
        let order: HashMap<TxnId, usize> = firsts
            .iter()
            .enumerate()
            .map(|(i, f)| (f.txn_id, i))
            .collect();
        groups.sort_by_key(|g| order[&g.0]);
    }
    let mut rows = Vec::new();
    let (mut debit, mut credit) = (Money::ZERO, Money::ZERO);
    for (n, (txn, fs)) in groups.iter().enumerate() {
        let (mut d, mut c) = (Money::ZERO, Money::ZERO);
        for f in fs {
            if f.amount.is_positive() {
                d += f.amount;
            } else {
                c += -f.amount;
            }
            rows.push(
                Row::new(
                    RowKind::Data,
                    1,
                    fact_cells(r, &splits, f, &cols, f.amount, None, Some(n + 1)),
                )
                .drill(Run::txn_drill(*txn)),
            );
        }
        rows.push(
            total_row(
                RowKind::Subtotal,
                1,
                String::new(),
                &cols,
                Totals {
                    debit: d,
                    credit: c,
                    amount: Money::ZERO,
                    balance: None,
                },
            )
            .drill(Run::txn_drill(*txn)),
        );
        debit += d;
        credit += c;
    }
    rows.push(total_row(
        RowKind::Total,
        0,
        "TOTAL".into(),
        &cols,
        Totals {
            debit,
            credit,
            amount: Money::ZERO,
            balance: None,
        },
    ));
    Ok(r.finish(r.range_line(), columns(&cols), rows, Vec::new()))
}

pub(crate) fn list_by_date(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let cols = [
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Name,
        DCol::Memo,
        DCol::Account,
        DCol::Clr,
        DCol::Split,
        DCol::Amount,
    ];
    let facts = r.ledger().facts(r.range)?;
    let splits = Splits::load(r, &facts)?;
    // One line per transaction: its header posting (the bank on a check, A/R on
    // an invoice), else its first.
    let mut main: Vec<Fact> = Vec::new();
    let mut index: HashMap<TxnId, usize> = HashMap::new();
    for f in facts {
        if let Some(&i) = index.get(&f.txn_id) {
            if main[i].line_id.is_some() && f.line_id.is_none() {
                main[i] = f;
            }
        } else {
            index.insert(f.txn_id, main.len());
            main.push(f);
        }
    }
    sort_facts(r, &mut main);
    let mut rows = Vec::new();
    for f in &main {
        let t = ctx.account_type(f.account_id).unwrap_or(AccountType::Bank);
        rows.push(
            Row::new(
                RowKind::Data,
                0,
                fact_cells(r, &splits, f, &cols, t.natural(f.amount), None, None),
            )
            .drill(Run::txn_drill(f.txn_id)),
        );
    }
    Ok(r.finish(r.range_line(), columns(&cols), rows, Vec::new()))
}
