//! The parity check: do LibreBooks' books say what QuickBooks' say?
//!
//! Two comparisons, both from figures QBD computes itself:
//!
//! 1. **Every account's balance.** The bridge's account list carries `Balance`
//!    (and `TotalBalance`, with subaccounts). Probing the live company showed what
//!    it means: for balance-sheet accounts, the balance now; for income and
//!    expense accounts, the **all-time** total since the company began (not this
//!    year's); always in the account's natural sign (a credit card owed is
//!    positive). So LibreBooks' side is the sum of all postings, ever.
//! 2. **The trial balance** as of a date, from QBD's own report. QBD's trial
//!    balance is "as of" only: balance-sheet accounts to the date, income and
//!    expense accounts for the fiscal year to date, and Retained Earnings holding
//!    every earlier year's net income plus its own postings.
//!
//! Always accrual basis, asked for explicitly: QBD's report default comes from a
//! company preference (the user's is Cash). A cash-basis comparison needs
//! LibreBooks' cash-basis engine (lb-reports) first.
//!
//! Where the bridge can't supply a transaction type (credit card charges,
//! transfers…), the accounts those touch are off by exactly what's missing; each
//! row that's off says which missing types could explain it.

use std::collections::HashMap;

use jiff::civil::Date;
use lb_core::{Account, AccountId, AccountRole, AccountType, Money};
use lb_store::{Books, StoreError, accounts, balances, company, extref};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::client::{Basis, BridgeError, ListQuery, QbdSource, ReportQuery};
use crate::entity::QbdEntity;
use crate::sync::bound_source;

/// What to compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParityOptions {
    /// The trial balance's date. `None`: today.
    pub as_of: Option<Date>,
    /// Also compare QBD's trial balance report (one more request).
    pub trial_balance: bool,
}

impl Default for ParityOptions {
    fn default() -> Self {
        ParityOptions {
            as_of: None,
            trial_balance: true,
        }
    }
}

/// One account, QBD's figure beside ours. Amounts are in the account's natural
/// sign (as QBD shows them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParityRow {
    pub account_id: Option<AccountId>,
    pub qbd_list_id: Option<String>,
    pub full_name: String,
    pub account_type: Option<AccountType>,
    pub qbd: Money,
    pub ours: Money,
    /// `ours − qbd`.
    pub difference: Money,
    /// With subaccounts (account balances only).
    pub qbd_total: Option<Money>,
    pub ours_total: Option<Money>,
    /// Why it might be off, or why it couldn't be compared.
    pub note: Option<String>,
}

impl ParityRow {
    pub fn matches(&self) -> bool {
        self.difference.is_zero() && self.qbd_total == self.ours_total
    }
}

/// QBD's trial balance beside ours.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrialBalanceParity {
    pub as_of: Date,
    pub basis: String,
    pub rows: Vec<ParityRow>,
    pub qbd_debits: Money,
    pub qbd_credits: Money,
    pub ours_debits: Money,
    pub ours_credits: Money,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParitySummary {
    pub accounts_compared: u32,
    pub accounts_matching: u32,
    pub accounts_off: u32,
    /// The sum of every account's absolute difference.
    pub total_abs_difference: Money,
    pub trial_balance_rows: u32,
    pub trial_balance_matching: u32,
    /// What the bridge can't supply, so what the differences may be.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParityReport {
    pub generated_at: jiff::Timestamp,
    pub source_id: String,
    pub accounts: Vec<ParityRow>,
    pub trial_balance: Option<TrialBalanceParity>,
    pub summary: ParitySummary,
}

#[derive(Debug, thiserror::Error)]
pub enum ParityError {
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("these books haven't been synced with QuickBooks yet")]
    NotSynced,
    #[error("QuickBooks' {0} didn't have the expected shape")]
    Garbled(&'static str),
}

/// Which transaction types the bridge doesn't expose could leave an account of
/// this type off.
pub fn likely_cause(t: AccountType) -> Option<&'static str> {
    Some(match t {
        AccountType::CreditCard => {
            "credit card charges and credits, and card payments, aren't exposed by the bridge"
        }
        AccountType::Bank => {
            "transfers, sales tax payments, paychecks and liability checks aren't exposed by the bridge"
        }
        AccountType::AccountsPayable => {
            "vendor credits, item receipts and credit card bill payments aren't exposed by the bridge"
        }
        AccountType::AccountsReceivable => {
            "statement charges aren't exposed by the bridge, and credits applied without a payment aren't visible"
        }
        AccountType::OtherCurrentLiability | AccountType::LongTermLiability => {
            "transfers, paychecks, liability checks and sales tax payments aren't exposed by the bridge"
        }
        AccountType::OtherCurrentAsset | AccountType::FixedAsset | AccountType::OtherAsset => {
            "transfers, inventory adjustments and item receipts aren't exposed by the bridge"
        }
        AccountType::Income | AccountType::OtherIncome => {
            "lines on credit card credits and statement charges aren't exposed by the bridge"
        }
        AccountType::Expense | AccountType::OtherExpense | AccountType::CostOfGoodsSold => {
            "lines on credit card charges, paychecks and item receipts aren't exposed by the bridge; COGS also depends on average cost"
        }
        AccountType::Equity => "transfers and journal-like transactions the bridge doesn't expose",
        AccountType::NonPosting => return None,
    })
}

fn money(v: Option<&Value>) -> Money {
    v.and_then(Value::as_str)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(Money::ZERO)
}

/// Compare the books with QBD's account balances and trial balance.
pub async fn parity<S: QbdSource>(
    books: &Books,
    source: &S,
    options: &ParityOptions,
) -> Result<ParityReport, ParityError> {
    let source_id = bound_source(books)?.ok_or(ParityError::NotSynced)?;
    let qbd_accounts = source
        .list(QbdEntity::Account, &ListQuery::default())
        .await?;
    let (local, links, all_time, info) = books.read(|c| {
        let local: HashMap<AccountId, Account> = accounts::list(c, true)?
            .into_iter()
            .map(|a| (a.id, a))
            .collect();
        let links: HashMap<String, AccountId> = extref::all_of_kind(c, &source_id, "Account")?
            .into_iter()
            .filter_map(|r| Some((r.external_id, lb_store::codec::parse_id(&r.local_id).ok()?)))
            .collect();
        Ok((
            local,
            links,
            balances::sums(c, None, None)?,
            company::info(c)?,
        ))
    })?;
    let natural = |id: AccountId, sums: &HashMap<AccountId, Money>| -> Money {
        let raw = sums.get(&id).copied().unwrap_or(Money::ZERO);
        local.get(&id).map_or(raw, |a| a.account_type.natural(raw))
    };
    // Subtree totals over LibreBooks' own hierarchy.
    let mut children: HashMap<AccountId, Vec<AccountId>> = HashMap::new();
    for a in local.values() {
        if let Some(p) = a.parent_id {
            children.entry(p).or_default().push(a.id);
        }
    }
    let subtree = |id: AccountId, sums: &HashMap<AccountId, Money>| -> Money {
        let mut total = Money::ZERO;
        let mut stack = vec![id];
        while let Some(x) = stack.pop() {
            total += natural(x, sums);
            if let Some(c) = children.get(&x) {
                stack.extend(c.iter().copied());
            }
        }
        total
    };

    // 1. Account balances.
    let mut rows = Vec::new();
    let mut seen_local = std::collections::HashSet::new();
    let mut qbd_by_name: HashMap<String, (String, Option<AccountType>)> = HashMap::new();
    for a in &qbd_accounts {
        let list_id = a
            .get("ListID")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let full_name = a
            .get("FullName")
            .or_else(|| a.get("Name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let account_type = a
            .get("AccountType")
            .and_then(Value::as_str)
            .and_then(AccountType::from_qbxml);
        qbd_by_name.insert(full_name.clone(), (list_id.clone(), account_type));
        if account_type == Some(AccountType::NonPosting) {
            continue;
        }
        let qbd = money(a.get("Balance"));
        let qbd_total = money(a.get("TotalBalance"));
        let row = match links.get(&list_id) {
            Some(id) => {
                seen_local.insert(*id);
                let ours = natural(*id, &all_time);
                let ours_total = subtree(*id, &all_time);
                let mut r = ParityRow {
                    account_id: Some(*id),
                    qbd_list_id: Some(list_id),
                    full_name,
                    account_type,
                    qbd,
                    ours,
                    difference: ours - qbd,
                    qbd_total: Some(qbd_total),
                    ours_total: Some(ours_total),
                    note: None,
                };
                if !r.matches() {
                    r.note = account_type.and_then(likely_cause).map(str::to_owned);
                }
                r
            }
            None => ParityRow {
                account_id: None,
                qbd_list_id: Some(list_id),
                full_name,
                account_type,
                qbd,
                ours: Money::ZERO,
                difference: -qbd,
                qbd_total: Some(qbd_total),
                ours_total: None,
                note: Some("not imported".into()),
            },
        };
        rows.push(row);
    }
    // LibreBooks accounts QBD doesn't know (created by the import or by hand)
    // that hold money.
    let mut extra: Vec<&Account> = local
        .values()
        .filter(|a| !seen_local.contains(&a.id) && a.account_type != AccountType::NonPosting)
        .filter(|a| !natural(a.id, &all_time).is_zero())
        .collect();
    extra.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    for a in extra {
        let ours = natural(a.id, &all_time);
        rows.push(ParityRow {
            account_id: Some(a.id),
            qbd_list_id: None,
            full_name: a.full_name.clone(),
            account_type: Some(a.account_type),
            qbd: Money::ZERO,
            ours,
            difference: ours,
            qbd_total: None,
            ours_total: None,
            note: Some("only in LibreBooks".into()),
        });
    }

    let mut summary = ParitySummary::default();
    for r in &rows {
        summary.accounts_compared += 1;
        if r.matches() {
            summary.accounts_matching += 1;
        } else {
            summary.accounts_off += 1;
            summary.total_abs_difference += r.difference.abs();
        }
    }
    let off_types: std::collections::BTreeSet<&str> = rows
        .iter()
        .filter(|r| !r.matches())
        .filter_map(|r| r.account_type.and_then(likely_cause))
        .collect();
    summary.notes = off_types.into_iter().map(str::to_owned).collect();

    // 2. Trial balance.
    let trial_balance = if options.trial_balance {
        let as_of = options.as_of.unwrap_or_else(|| jiff::Zoned::now().date());
        let report = source
            .report(
                "trial-balance",
                &ReportQuery {
                    to_date: Some(as_of),
                    basis: Some(Basis::Accrual),
                    ..ReportQuery::default()
                },
            )
            .await?;
        let fy_start = lb_core::period::fiscal_year(as_of, info.fiscal_year_start_month, 0).0;
        let before_fy = fy_start.yesterday().unwrap_or(fy_start);
        let (to_date, this_year, earlier) = books.read(|c| {
            Ok((
                balances::sums(c, None, Some(as_of))?,
                balances::sums(c, Some(fy_start), Some(as_of))?,
                balances::sums(c, None, Some(before_fy))?,
            ))
        })?;
        let tb = trial_balance_parity(
            &report,
            as_of,
            &local,
            &links,
            &qbd_by_name,
            &to_date,
            &this_year,
            &earlier,
        )?;
        summary.trial_balance_rows = tb.rows.len() as u32;
        summary.trial_balance_matching =
            tb.rows.iter().filter(|r| r.difference.is_zero()).count() as u32;
        Some(tb)
    } else {
        None
    };

    Ok(ParityReport {
        generated_at: jiff::Timestamp::now(),
        source_id,
        accounts: rows,
        trial_balance,
        summary,
    })
}

/// Our trial balance as QBD builds its own, beside QBD's rows.
#[allow(clippy::too_many_arguments)]
fn trial_balance_parity(
    report: &Value,
    as_of: Date,
    local: &HashMap<AccountId, Account>,
    links: &HashMap<String, AccountId>,
    qbd_by_name: &HashMap<String, (String, Option<AccountType>)>,
    to_date: &HashMap<AccountId, Money>,
    this_year: &HashMap<AccountId, Money>,
    earlier: &HashMap<AccountId, Money>,
) -> Result<TrialBalanceParity, ParityError> {
    let rows_v = report
        .get("rows")
        .and_then(Value::as_array)
        .ok_or(ParityError::Garbled("trial balance"))?;
    let basis = report
        .get("basis")
        .and_then(Value::as_str)
        .unwrap_or("Accrual")
        .to_owned();

    // Ours, debit-positive, per account.
    let retained = local
        .values()
        .find(|a| a.role == Some(AccountRole::RetainedEarnings))
        .map(|a| a.id);
    let mut ours: HashMap<AccountId, Money> = HashMap::new();
    let mut closed_into_retained = Money::ZERO;
    for a in local.values() {
        let get = |m: &HashMap<AccountId, Money>| m.get(&a.id).copied().unwrap_or(Money::ZERO);
        if a.account_type.is_income_statement() {
            ours.insert(a.id, get(this_year));
            closed_into_retained += get(earlier);
        } else if a.account_type.is_balance_sheet() {
            ours.insert(a.id, get(to_date));
        }
    }
    if let Some(re) = retained {
        *ours.entry(re).or_insert(Money::ZERO) += closed_into_retained;
    }

    let mut out = Vec::new();
    let mut matched = std::collections::HashSet::new();
    let (mut qbd_debits, mut qbd_credits) = (Money::ZERO, Money::ZERO);
    for r in rows_v {
        let values = r.get("values").cloned().unwrap_or(Value::Null);
        let debit = money(values.get("2"));
        let credit = money(values.get("3"));
        if r.get("type").and_then(Value::as_str) == Some("TotalRow") {
            qbd_debits = debit;
            qbd_credits = credit;
            continue;
        }
        if r.get("type").and_then(Value::as_str) != Some("DataRow") {
            continue;
        }
        let label = r
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let qbd_dr = debit - credit;
        let found = qbd_by_name.get(&label);
        let account_type = found.and_then(|(_, t)| *t);
        let nat = |m: Money| account_type.map_or(m, |t| t.natural(m));
        let local_id = found.and_then(|(lid, _)| links.get(lid)).copied();
        let row = match local_id {
            Some(id) => {
                matched.insert(id);
                let o = ours.get(&id).copied().unwrap_or(Money::ZERO);
                let mut row = ParityRow {
                    account_id: Some(id),
                    qbd_list_id: found.map(|(l, _)| l.clone()),
                    full_name: label,
                    account_type,
                    qbd: nat(qbd_dr),
                    ours: nat(o),
                    difference: nat(o) - nat(qbd_dr),
                    qbd_total: None,
                    ours_total: None,
                    note: None,
                };
                if !row.difference.is_zero() {
                    row.note = account_type.and_then(likely_cause).map(str::to_owned);
                }
                row
            }
            None => ParityRow {
                account_id: None,
                qbd_list_id: found.map(|(l, _)| l.clone()),
                full_name: label,
                account_type,
                qbd: nat(qbd_dr),
                ours: Money::ZERO,
                difference: -nat(qbd_dr),
                qbd_total: None,
                ours_total: None,
                note: Some("not imported".into()),
            },
        };
        out.push(row);
    }
    // Accounts with a balance here that QBD's trial balance leaves out.
    let mut missing: Vec<(&AccountId, &Money)> = ours
        .iter()
        .filter(|(id, m)| !m.is_zero() && !matched.contains(*id))
        .collect();
    missing.sort_by_key(|(id, _)| local.get(id).map(|a| a.full_name.clone()));
    for (id, m) in missing {
        let a = local.get(id);
        let t = a.map(|a| a.account_type);
        let nat = t.map_or(*m, |t| t.natural(*m));
        out.push(ParityRow {
            account_id: Some(*id),
            qbd_list_id: None,
            full_name: a.map(|a| a.full_name.clone()).unwrap_or_default(),
            account_type: t,
            qbd: Money::ZERO,
            ours: nat,
            difference: nat,
            qbd_total: None,
            ours_total: None,
            note: Some("not on QuickBooks' trial balance".into()),
        });
    }
    let ours_debits = ours.values().filter(|m| m.is_positive()).copied().sum();
    let ours_credits = -ours
        .values()
        .filter(|m| m.is_negative())
        .copied()
        .sum::<Money>();
    Ok(TrialBalanceParity {
        as_of,
        basis,
        rows: out,
        qbd_debits,
        qbd_credits,
        ours_debits,
        ours_credits,
    })
}
