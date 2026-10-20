//! Renaming and categorising rules (QBD's "renaming rules", plus what to fill in
//! when adding).
//!
//! A rule has conditions (the payee contains / equals / starts with / matches a
//! pattern, the amount is or is between, money in or out, which account) and
//! actions (show the payee as, use this name, this account, this class, this
//! memo, and optionally add it without asking). Rules are tried in priority
//! order and **the first that matches wins**, so a person can always predict
//! which one applies; QBD's renaming rules behave the same way.
//!
//! When no rule matches, the payee's history fills in: if a transaction with the
//! same cleaned-up payee, going the same way, was added before, the new one is
//! offered the same name and account. That gives most of the benefit of rules
//! without anyone writing one.

use std::collections::HashMap;

use lb_core::{AccountId, ClassId, EntityId, LineKind, Money, TxnId};
use lb_store::{Writer, accounts, audit, codec, entities, txns};
use regex::{Regex, RegexBuilder};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{FeedError, Result};
use crate::ids::{FeedRuleId, FeedTxnId};
use crate::queue::{self, FeedFilter, FeedStatus, FeedTxn, Suggestion};
use crate::statement::Direction;
use crate::text;

/// Which text a condition looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TextField {
    Payee,
    Memo,
    /// Either one.
    PayeeOrMemo,
}

/// How a condition compares text. Always case-insensitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TextOp {
    Contains,
    Equals,
    StartsWith,
    /// A regular expression (Rust `regex` syntax; no look-around, linear time).
    Regex,
}

/// One test a downloaded transaction must pass.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum Condition {
    Text {
        field: TextField,
        op: TextOp,
        value: String,
    },
    /// The amount, ignoring its sign, is exactly this.
    AmountEquals {
        amount: Money,
    },
    /// The amount, ignoring its sign, is within this range (either end open,
    /// both inclusive).
    AmountRange {
        min: Option<Money>,
        max: Option<Money>,
    },
    Direction {
        direction: Direction,
    },
    /// The downloaded transaction is in this account.
    Account {
        account_id: AccountId,
    },
}

/// What a rule does to the transactions it matches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct RuleActions {
    /// Show the payee as this.
    pub rename_to: Option<String>,
    /// Put this name (an existing vendor, customer…) on the transaction.
    pub entity_id: Option<EntityId>,
    /// The account on the other side.
    pub account_id: Option<AccountId>,
    pub class_id: Option<ClassId>,
    pub memo: Option<String>,
    /// Add it without asking when nothing in the register matches it. Needs an
    /// account.
    pub auto_add: bool,
}

/// A rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeedRule {
    pub id: FeedRuleId,
    pub name: String,
    /// Lower runs first.
    pub priority: i64,
    pub enabled: bool,
    /// All conditions must hold (true) or any one (false).
    pub match_all: bool,
    pub conditions: Vec<Condition>,
    pub actions: RuleActions,
    #[serde(default)]
    pub edit_seq: i64,
}

impl FeedRule {
    pub fn new(name: &str, conditions: Vec<Condition>, actions: RuleActions) -> FeedRule {
        FeedRule {
            id: FeedRuleId::new(),
            name: name.to_owned(),
            priority: 0,
            enabled: true,
            match_all: true,
            conditions,
            actions,
            edit_seq: 0,
        }
    }
}

fn compile(pattern: &str) -> Result<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| FeedError::Invalid(format!("the pattern {pattern:?} isn't valid: {e}")))
}

/// Rules ready to run: in order, patterns compiled once.
#[derive(Debug, Default)]
pub struct RuleSet {
    rules: Vec<(FeedRule, Vec<Option<Regex>>)>,
}

impl RuleSet {
    pub fn new(rules: Vec<FeedRule>) -> Result<RuleSet> {
        let mut out = Vec::new();
        for r in rules.into_iter().filter(|r| r.enabled) {
            let regexes = r
                .conditions
                .iter()
                .map(|c| match c {
                    Condition::Text {
                        op: TextOp::Regex,
                        value,
                        ..
                    } => compile(value).map(Some),
                    _ => Ok(None),
                })
                .collect::<Result<Vec<_>>>()?;
            out.push((r, regexes));
        }
        out.sort_by_key(|(r, _)| r.priority);
        Ok(RuleSet { rules: out })
    }

    /// Every enabled rule in the books.
    pub fn load(c: &Connection) -> Result<RuleSet> {
        RuleSet::new(list(c)?)
    }

    /// The first rule that matches, if any.
    pub fn first_match(&self, t: &FeedTxn) -> Option<&FeedRule> {
        self.rules
            .iter()
            .find(|(r, re)| rule_matches(r, re, t))
            .map(|(r, _)| r)
    }
}

fn rule_matches(r: &FeedRule, regexes: &[Option<Regex>], t: &FeedTxn) -> bool {
    if r.conditions.is_empty() {
        return false;
    }
    let mut results = r
        .conditions
        .iter()
        .zip(regexes)
        .map(|(c, re)| condition_matches(c, re.as_ref(), t));
    if r.match_all {
        results.all(|x| x)
    } else {
        results.any(|x| x)
    }
}

/// Does one condition hold for a downloaded transaction?
pub fn condition_matches(c: &Condition, re: Option<&Regex>, t: &FeedTxn) -> bool {
    match c {
        Condition::Text { field, op, value } => {
            let test = |s: Option<&str>| {
                let Some(s) = s else { return false };
                let hay = s.trim().to_lowercase();
                let needle = value.trim().to_lowercase();
                match op {
                    TextOp::Contains => !needle.is_empty() && hay.contains(&needle),
                    TextOp::Equals => hay == needle,
                    TextOp::StartsWith => !needle.is_empty() && hay.starts_with(&needle),
                    TextOp::Regex => re.is_some_and(|re| re.is_match(s)),
                }
            };
            match field {
                TextField::Payee => test(t.payee.as_deref()),
                TextField::Memo => test(t.memo.as_deref()),
                TextField::PayeeOrMemo => test(t.payee.as_deref()) || test(t.memo.as_deref()),
            }
        }
        Condition::AmountEquals { amount } => t.amount.abs() == amount.abs(),
        Condition::AmountRange { min, max } => {
            let a = t.amount.abs();
            min.is_none_or(|m| a >= m.abs()) && max.is_none_or(|m| a <= m.abs())
        }
        Condition::Direction { direction } => Direction::of(t.amount) == *direction,
        Condition::Account { account_id } => t.account_id == *account_id,
    }
}

/// Every rule, in the order they run.
pub fn list(c: &Connection) -> Result<Vec<FeedRule>> {
    let mut stmt = c.prepare("SELECT doc FROM feed_rule ORDER BY priority, name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| codec::doc::<FeedRule>(r, 0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn find(c: &Connection, id: FeedRuleId) -> Result<Option<FeedRule>> {
    Ok(c.query_row(
        "SELECT doc FROM feed_rule WHERE id = ?1",
        [id.to_string()],
        |r| codec::doc(r, 0),
    )
    .optional()?)
}

pub fn get(c: &Connection, id: FeedRuleId) -> Result<FeedRule> {
    find(c, id)?.ok_or_else(|| FeedError::NotFound {
        kind: "rule",
        id: id.to_string(),
    })
}

/// Create or update a rule, checking that it can run and that what it points at
/// exists. A new rule with priority 0 goes after the existing ones.
pub fn save(w: &mut Writer<'_>, mut r: FeedRule) -> Result<FeedRule> {
    let existing = find(&w.tx, r.id)?;
    match &existing {
        Some(x) if x.edit_seq != r.edit_seq => {
            return Err(lb_store::StoreError::Conflict {
                kind: "rule",
                id: r.id.to_string(),
                expected: r.edit_seq,
                actual: x.edit_seq,
            }
            .into());
        }
        None if r.edit_seq != 0 => {
            return Err(FeedError::NotFound {
                kind: "rule",
                id: r.id.to_string(),
            });
        }
        None if r.priority == 0 => {
            let max: Option<i64> =
                w.tx.query_row("SELECT MAX(priority) FROM feed_rule", [], |row| row.get(0))?;
            r.priority = max.unwrap_or(0) + 10;
        }
        _ => {}
    }
    validate(&w.tx, &r)?;
    r.name = r.name.trim().to_owned();
    r.edit_seq += 1;
    let id = r.id.to_string();
    w.tx.execute(
        "INSERT INTO feed_rule (id, priority, name, doc) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET priority = excluded.priority, name = excluded.name, doc = excluded.doc",
        params![id, r.priority, r.name, codec::to_doc(&r)?],
    )?;
    let action = if existing.is_some() {
        "update"
    } else {
        "create"
    };
    audit::record(
        w,
        "feed_rule",
        &id,
        action,
        existing.as_ref(),
        Some(&r),
        None,
    )?;
    w.changed("feed_rule", id, action);
    Ok(r)
}

fn validate(c: &Connection, r: &FeedRule) -> Result<()> {
    if r.name.trim().is_empty() {
        return Err(FeedError::Invalid("give the rule a name".into()));
    }
    if r.conditions.is_empty() {
        return Err(FeedError::Invalid(
            "a rule needs at least one condition".into(),
        ));
    }
    for cond in &r.conditions {
        match cond {
            Condition::Text { op, value, .. } => {
                if value.trim().is_empty() && *op != TextOp::Equals {
                    return Err(FeedError::Invalid(
                        "a text condition needs some text".into(),
                    ));
                }
                if *op == TextOp::Regex {
                    compile(value)?;
                }
            }
            Condition::AmountRange { min, max } => {
                if let (Some(a), Some(b)) = (min, max)
                    && a.abs() > b.abs()
                {
                    return Err(FeedError::Invalid(
                        "the amount range's minimum is above its maximum".into(),
                    ));
                }
            }
            Condition::Account { account_id } => {
                accounts::get(c, *account_id)?;
            }
            Condition::AmountEquals { .. } | Condition::Direction { .. } => {}
        }
    }
    let a = &r.actions;
    if let Some(e) = a.entity_id {
        entities::get(c, e)?;
    }
    if let Some(acct) = a.account_id {
        accounts::get(c, acct)?;
    }
    if let Some(class) = a.class_id {
        lb_store::lists::find::<lb_core::lists::Class>(c, &class.to_string())?.ok_or_else(
            || FeedError::NotFound {
                kind: "class",
                id: class.to_string(),
            },
        )?;
    }
    if a.auto_add && a.account_id.is_none() {
        return Err(FeedError::Invalid(
            "to add transactions automatically, the rule must choose an account".into(),
        ));
    }
    Ok(())
}

pub fn delete(w: &mut Writer<'_>, id: FeedRuleId) -> Result<()> {
    let r = get(&w.tx, id)?;
    let sid = id.to_string();
    w.tx.execute("DELETE FROM feed_rule WHERE id = ?1", [&sid])?;
    audit::record::<FeedRule, FeedRule>(w, "feed_rule", &sid, "delete", Some(&r), None, None)?;
    w.changed("feed_rule", sid, "delete");
    Ok(())
}

/// Put rules in this order (the rule list's drag-to-reorder). Rules not named
/// keep their relative order after the named ones.
pub fn reorder(w: &mut Writer<'_>, ids: &[FeedRuleId]) -> Result<()> {
    let mut all = list(&w.tx)?;
    all.sort_by_key(|r| ids.iter().position(|i| *i == r.id).unwrap_or(usize::MAX));
    for (n, r) in all.into_iter().enumerate() {
        let p = (n as i64 + 1) * 10;
        if r.priority != p {
            let mut r = r;
            r.priority = p;
            w.tx.execute(
                "UPDATE feed_rule SET priority = ?2, doc = ?3 WHERE id = ?1",
                params![r.id.to_string(), p, codec::to_doc(&r)?],
            )?;
        }
    }
    w.changed("feed_rule", "*", "reorder");
    Ok(())
}

/// How payees were added before: cleaned payee and direction → the transaction
/// that was made.
#[derive(Debug, Default)]
pub struct History {
    by_payee: HashMap<(String, Direction), TxnId>,
}

impl History {
    /// The most recent added transaction for each payee and direction, from the
    /// last few thousand added rows.
    pub fn load(c: &Connection) -> Result<History> {
        let mut stmt = c.prepare(
            "SELECT payee, memo, amount, matched_txn_id FROM feed_txn
             WHERE status = 'added' AND matched_txn_id IS NOT NULL
             ORDER BY posted_date DESC, rowid DESC LIMIT 5000",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut h = History::default();
        for row in rows {
            let (payee, memo, amount, txn) = row?;
            let Some(key) = history_key(payee.as_deref().or(memo.as_deref()), Money(amount)) else {
                continue;
            };
            h.by_payee.entry(key).or_insert(codec::parse_id(&txn)?);
        }
        Ok(h)
    }

    /// The suggestion history gives for a downloaded transaction.
    pub fn suggest(&self, c: &Connection, t: &FeedTxn) -> Result<Option<Suggestion>> {
        let Some(key) = history_key(t.payee.as_deref().or(t.memo.as_deref()), t.amount) else {
            return Ok(None);
        };
        let Some(&txn_id) = self.by_payee.get(&key) else {
            return Ok(None);
        };
        let Some(txn) = txns::find(c, txn_id)? else {
            return Ok(None);
        };
        let line = txn.lines.iter().find(|l| {
            matches!(
                l.kind,
                LineKind::Account | LineKind::DepositLine | LineKind::Item
            ) && l.account_id.is_some()
        });
        let entity_id = txn.entity_id.or_else(|| line.and_then(|l| l.entity_id));
        let account_id = line.and_then(|l| l.account_id);
        if entity_id.is_none() && account_id.is_none() {
            return Ok(None);
        }
        Ok(Some(Suggestion {
            from_history: true,
            entity_id,
            account_id,
            class_id: line.and_then(|l| l.class_id).or(txn.class_id),
            ..Suggestion::default()
        }))
    }
}

fn history_key(payee: Option<&str>, amount: Money) -> Option<(String, Direction)> {
    let cleaned = text::clean_payee(payee?);
    (!cleaned.is_empty()).then(|| (cleaned, Direction::of(amount)))
}

/// What a rule suggests for a transaction.
pub fn suggestion_from(rule: &FeedRule) -> Suggestion {
    let a = &rule.actions;
    Suggestion {
        rule_id: Some(rule.id),
        from_history: false,
        payee: a.rename_to.clone(),
        entity_id: a.entity_id,
        account_id: a.account_id,
        class_id: a.class_id,
        memo: a.memo.clone(),
        auto_add: a.auto_add,
    }
}

/// Work out (and store) the suggestion for each of these waiting transactions:
/// the first matching rule's, else history's, else none. Returns how many got
/// one.
pub fn apply_to(w: &mut Writer<'_>, ids: &[FeedTxnId]) -> Result<u32> {
    let rules = RuleSet::load(&w.tx)?;
    let history = History::load(&w.tx)?;
    let mut n = 0;
    for &id in ids {
        let t = queue::get(&w.tx, id)?;
        if t.status != FeedStatus::New {
            continue;
        }
        let s = match rules.first_match(&t) {
            Some(r) => Some(suggestion_from(r)),
            None => history.suggest(&w.tx, &t)?,
        };
        if s.is_some() {
            n += 1;
        }
        if s != t.suggestion {
            queue::set_suggestion(w, id, s)?;
        }
    }
    Ok(n)
}

/// Re-run the rules over everything still waiting (after a rule is added or
/// changed), optionally in one account.
pub fn apply_all(w: &mut Writer<'_>, account: Option<AccountId>) -> Result<u32> {
    let waiting = queue::query(
        &w.tx,
        &FeedFilter {
            account_id: account,
            statuses: vec![FeedStatus::New],
            limit: Some(10_000),
            ..FeedFilter::default()
        },
    )?;
    let ids: Vec<FeedTxnId> = waiting.iter().map(|t| t.id).collect();
    apply_to(w, &ids)
}
