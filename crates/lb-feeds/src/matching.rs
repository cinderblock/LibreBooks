//! Matching downloaded transactions to what is already in the register (QBD's
//! "Match" in Bank Feeds).
//!
//! A candidate is a register transaction in the same account whose effect on
//! that account is exactly the downloaded amount (sign included: a deposit never
//! matches a check), dated within a window either side of the bank's date, not
//! void, not already cleared, and not already linked to another download. Each
//! candidate is scored:
//!
//! | Signal | Points |
//! | --- | --- |
//! | exact amount (required) | 40 |
//! | date distance `d` of a window of `N` days | `30 × (1 − d/(N+1))` |
//! | check numbers equal | +30 |
//! | check numbers both present and different | −40 |
//! | payee similarity (or the rule's name is the transaction's name) | up to +20 |
//!
//! A candidate scoring [`STRONG`] or more is strong. A download is matched
//! automatically only when it has exactly one strong candidate, and that
//! candidate isn't also the only strong candidate of another download in the
//! same batch (two identical $50 transfers in one week wait for a person).
//!
//! The amount works for every account type because the feed's sign convention
//! (positive is money in) is the register's debit-positive posting in that
//! account: a deposit debits a bank, and a card payment debits the card's
//! liability.

use std::collections::HashMap;

use jiff::civil::Date;
use lb_core::{ClearedStatus, EntityId, Money, TxnId, TxnType};
use lb_store::codec::{self, SqlDate};
use lb_store::{Writer, balances};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{FeedError, Result};
use crate::ids::FeedTxnId;
use crate::queue::{self, FeedStatus, FeedTxn};
use crate::text;

/// The score at which a candidate is strong enough to match without asking.
pub const STRONG: u8 = 65;

/// How wide to look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MatchOptions {
    /// Days either side of the bank's date.
    pub days: u32,
}

impl Default for MatchOptions {
    fn default() -> Self {
        MatchOptions { days: 10 }
    }
}

/// A register transaction that might be the downloaded one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MatchCandidate {
    pub txn_id: TxnId,
    pub txn_type: TxnType,
    pub date: Date,
    pub number: Option<String>,
    pub entity_id: Option<EntityId>,
    /// The names on the transaction (header and lines).
    pub names: Option<String>,
    pub memo: Option<String>,
    /// Its effect on the account, debit-positive (equal to the download's amount).
    pub amount: Money,
    /// 0–100.
    pub score: u8,
    pub strong: bool,
    /// Why it scored what it did, for the person choosing.
    pub reasons: Vec<String>,
}

/// What [`score`] looks at: one download against one register transaction.
#[derive(Clone, Copy, Debug)]
pub struct ScoreInput<'a> {
    pub feed_amount: Money,
    pub feed_date: Date,
    pub feed_check: Option<&'a str>,
    /// The payee the download shows (a rule's rename, else the bank's text).
    pub feed_payee: Option<&'a str>,
    /// The name a rule chose for the download, if any.
    pub feed_entity: Option<EntityId>,
    pub txn_amount: Money,
    pub txn_date: Date,
    pub txn_type: TxnType,
    pub txn_number: Option<&'a str>,
    pub txn_entity: Option<EntityId>,
    /// Names and memo on the register transaction, for similarity.
    pub txn_text: &'a str,
    pub window_days: u32,
}

const fn is_check_like(t: TxnType) -> bool {
    matches!(
        t,
        TxnType::Check
            | TxnType::BillPaymentCheck
            | TxnType::Paycheck
            | TxnType::LiabilityCheck
            | TxnType::SalesTaxPaymentCheck
    )
}

fn check_digits(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .then(|| s.trim_start_matches('0').to_owned())
}

/// Score one candidate, or `None` if it can't be the same transaction (another
/// amount, or outside the window).
pub fn score(i: &ScoreInput<'_>) -> Option<(u8, Vec<String>)> {
    if i.feed_amount != i.txn_amount {
        return None;
    }
    let days = i64::from((i.txn_date - i.feed_date).get_days()).unsigned_abs();
    if days > u64::from(i.window_days) {
        return None;
    }
    let mut reasons = vec!["same amount".to_owned()];
    let mut points: i32 = 40;
    let window = f64::from(i.window_days) + 1.0;
    let date_points = (30.0 * (1.0 - days as f64 / window)).round() as i32;
    points += date_points;
    reasons.push(match days {
        0 => "same date".to_owned(),
        1 => "1 day apart".to_owned(),
        d => format!("{d} days apart"),
    });
    let feed_check = i.feed_check.and_then(check_digits);
    let txn_check = if is_check_like(i.txn_type) {
        i.txn_number.and_then(check_digits)
    } else {
        None
    };
    match (&feed_check, &txn_check) {
        (Some(a), Some(b)) if a == b => {
            points += 30;
            reasons.push(format!("check number {a}"));
        }
        (Some(a), Some(b)) => {
            points -= 40;
            reasons.push(format!("check number {a} vs {b}"));
        }
        _ => {}
    }
    if i.feed_entity.is_some() && i.feed_entity == i.txn_entity {
        points += 20;
        reasons.push("same name".to_owned());
    } else if let Some(p) = i.feed_payee {
        let sim = text::similarity(p, i.txn_text);
        let payee_points = (20.0 * sim).round() as i32;
        if payee_points > 0 {
            points += payee_points;
            reasons.push(if sim >= 0.99 {
                "same payee".to_owned()
            } else {
                "similar payee".to_owned()
            });
        }
    }
    Some((points.clamp(0, 100) as u8, reasons))
}

/// Register transactions that might be this download, best first.
pub fn candidates(c: &Connection, f: &FeedTxn, opts: MatchOptions) -> Result<Vec<MatchCandidate>> {
    let span = jiff::Span::new().days(i64::from(opts.days));
    let from = f.posted.checked_sub(span).unwrap_or(f.posted);
    let to = f.posted.checked_add(span).unwrap_or(f.posted);
    let mut stmt = c.prepare(
        "SELECT t.id, t.txn_type, t.date, t.number, t.entity_id, t.memo, SUM(p.amount),
                (SELECT group_concat(DISTINCT e2.full_name) FROM posting p2 JOIN entity e2 ON e2.id = p2.entity_id
                  WHERE p2.txn_id = t.id) AS line_names,
                e.full_name,
                (SELECT group_concat(DISTINCT p3.memo) FROM posting p3 WHERE p3.txn_id = t.id AND p3.memo IS NOT NULL)
         FROM posting p JOIN txn t ON t.id = p.txn_id LEFT JOIN entity e ON e.id = t.entity_id
         WHERE p.account_id = ?1 AND p.date >= ?2 AND p.date <= ?3 AND t.is_void = 0 AND t.is_pending = 0
           AND NOT EXISTS (SELECT 1 FROM feed_txn f
                           WHERE f.matched_txn_id = t.id AND f.status IN ('matched', 'added') AND f.id <> ?5)
         GROUP BY t.id
         HAVING SUM(p.amount) = ?4 AND SUM(p.cleared <> 'Uncleared') = 0",
    )?;
    let rows = stmt.query_map(
        params![
            f.account_id.to_string(),
            SqlDate(from),
            SqlDate(to),
            f.amount.minor(),
            f.id.to_string()
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, SqlDate>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
            ))
        },
    )?;
    let suggestion = f.suggestion.as_ref();
    let feed_payee = suggestion
        .and_then(|s| s.payee.as_deref())
        .or(f.payee.as_deref())
        .or(f.memo.as_deref());
    let mut out = Vec::new();
    for row in rows {
        let (id, kind, date, number, entity, memo, amount, line_names, header_name, posting_memos) =
            row?;
        let txn_type = TxnType::parse(&kind)
            .ok_or_else(|| lb_store::StoreError::Corrupt(format!("type {kind}")))?;
        let entity_id: Option<EntityId> = entity.as_deref().map(codec::parse_id).transpose()?;
        let names = match (header_name, line_names) {
            (Some(head), Some(lines)) if lines.contains(&head) => Some(lines),
            (Some(head), Some(lines)) => Some(format!("{head},{lines}")),
            (head, lines) => head.or(lines),
        };
        let txn_text = [names.as_deref(), memo.as_deref(), posting_memos.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let input = ScoreInput {
            feed_amount: f.amount,
            feed_date: f.posted,
            feed_check: f.check_number.as_deref(),
            feed_payee,
            feed_entity: suggestion.and_then(|s| s.entity_id),
            txn_amount: Money(amount),
            txn_date: date.0,
            txn_type,
            txn_number: number.as_deref(),
            txn_entity: entity_id,
            txn_text: &txn_text,
            window_days: opts.days,
        };
        if let Some((score, reasons)) = score(&input) {
            out.push(MatchCandidate {
                txn_id: codec::parse_id(&id)?,
                txn_type,
                date: date.0,
                number,
                entity_id,
                names,
                memo,
                amount: Money(amount),
                score,
                strong: score >= STRONG,
                reasons,
            });
        }
    }
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.date.cmp(&b.date))
            .then_with(|| a.txn_id.cmp(&b.txn_id))
    });
    Ok(out)
}

/// Link a download to a register transaction and mark that transaction cleared
/// in the account.
pub fn accept(w: &mut Writer<'_>, feed: FeedTxnId, txn: TxnId) -> Result<()> {
    let f = queue::get(&w.tx, feed)?;
    if f.status != FeedStatus::New {
        return Err(FeedError::Invalid(format!(
            "this downloaded transaction is already {}; undo that first",
            f.status.as_str()
        )));
    }
    let (effect, cleared, is_void): (Option<i64>, i64, bool) =
        w.tx.query_row(
            "SELECT SUM(p.amount), SUM(p.cleared <> 'Uncleared'), t.is_void
         FROM txn t LEFT JOIN posting p ON p.txn_id = t.id AND p.account_id = ?2
         WHERE t.id = ?1 GROUP BY t.id",
            params![txn.to_string(), f.account_id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    r.get(2)?,
                ))
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => FeedError::NotFound {
                kind: "transaction",
                id: txn.to_string(),
            },
            e => e.into(),
        })?;
    if is_void {
        return Err(FeedError::Invalid(
            "a void transaction can't be matched".into(),
        ));
    }
    let Some(effect) = effect else {
        return Err(FeedError::Invalid(
            "that transaction isn't in this account".into(),
        ));
    };
    if effect != f.amount.minor() {
        return Err(FeedError::Invalid(format!(
            "the amounts differ: the bank says {}, the register says {}",
            f.amount,
            Money(effect)
        )));
    }
    if cleared > 0 {
        return Err(FeedError::Invalid(
            "that transaction is already cleared in this account".into(),
        ));
    }
    if let Some(other) = queue::linked_to(&w.tx, txn)?
        && other != feed
    {
        return Err(FeedError::Invalid(
            "that transaction is already matched to another downloaded transaction".into(),
        ));
    }
    balances::set_cleared(w, txn, f.account_id, ClearedStatus::Cleared)?;
    queue::set_status(w, feed, FeedStatus::Matched, Some(txn))?;
    Ok(())
}

/// A download and the register transaction it was linked to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MatchedPair {
    pub feed_txn_id: FeedTxnId,
    pub txn_id: TxnId,
}

/// Match each of these waiting downloads that has exactly one strong candidate
/// (and doesn't share it with another download here).
pub fn auto_match(
    w: &mut Writer<'_>,
    ids: &[FeedTxnId],
    opts: MatchOptions,
) -> Result<Vec<MatchedPair>> {
    let mut proposals: Vec<MatchedPair> = Vec::new();
    for &id in ids {
        let f = queue::get(&w.tx, id)?;
        if f.status != FeedStatus::New {
            continue;
        }
        let cands = candidates(&w.tx, &f, opts)?;
        let mut strong = cands.iter().filter(|c| c.strong);
        if let (Some(only), None) = (strong.next(), strong.next()) {
            proposals.push(MatchedPair {
                feed_txn_id: id,
                txn_id: only.txn_id,
            });
        }
    }
    let mut claims: HashMap<TxnId, u32> = HashMap::new();
    for p in &proposals {
        *claims.entry(p.txn_id).or_default() += 1;
    }
    let mut out = Vec::new();
    for p in proposals {
        if claims.get(&p.txn_id) == Some(&1) {
            accept(w, p.feed_txn_id, p.txn_id)?;
            out.push(p);
        }
    }
    Ok(out)
}

/// For each waiting download: does anything in the register look like it at
/// all? (Auto-add never adds one that does.)
pub(crate) fn has_any_candidate(c: &Connection, f: &FeedTxn, opts: MatchOptions) -> Result<bool> {
    Ok(!candidates(c, f, opts)?.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn input<'a>() -> ScoreInput<'a> {
        ScoreInput {
            feed_amount: Money(-12_000),
            feed_date: date(2026, 10, 10),
            feed_check: None,
            feed_payee: None,
            feed_entity: None,
            txn_amount: Money(-12_000),
            txn_date: date(2026, 10, 10),
            txn_type: TxnType::Check,
            txn_number: None,
            txn_entity: None,
            txn_text: "",
            window_days: 10,
        }
    }

    #[test]
    fn scoring_table() {
        let e = EntityId::new();
        // (what, input, expected score or None)
        let cases: Vec<(&str, ScoreInput<'_>, Option<u8>)> = vec![
            ("same amount, same day", input(), Some(70)),
            (
                "opposite sign never matches",
                ScoreInput {
                    txn_amount: Money(12_000),
                    ..input()
                },
                None,
            ),
            (
                "a cent off never matches",
                ScoreInput {
                    txn_amount: Money(-11_999),
                    ..input()
                },
                None,
            ),
            (
                "outside the window",
                ScoreInput {
                    txn_date: date(2026, 10, 21),
                    ..input()
                },
                None,
            ),
            (
                "edge of the window",
                ScoreInput {
                    txn_date: date(2026, 10, 20),
                    ..input()
                },
                Some(43),
            ),
            (
                "three days earlier",
                ScoreInput {
                    txn_date: date(2026, 10, 7),
                    ..input()
                },
                Some(62),
            ),
            (
                "check number agrees",
                ScoreInput {
                    feed_check: Some("1041"),
                    txn_number: Some("1041"),
                    txn_date: date(2026, 10, 3),
                    ..input()
                },
                Some(81),
            ),
            (
                "leading zeros don't matter",
                ScoreInput {
                    feed_check: Some("0001041"),
                    txn_number: Some("1041"),
                    ..input()
                },
                Some(100),
            ),
            (
                "check number disagrees",
                ScoreInput {
                    feed_check: Some("1041"),
                    txn_number: Some("1042"),
                    ..input()
                },
                Some(30),
            ),
            (
                "a deposit's number isn't a check number",
                ScoreInput {
                    feed_check: Some("1041"),
                    txn_number: Some("1042"),
                    txn_type: TxnType::Deposit,
                    feed_amount: Money(12_000),
                    txn_amount: Money(12_000),
                    ..input()
                },
                Some(70),
            ),
            (
                "similar payee",
                ScoreInput {
                    feed_payee: Some("COMCAST CABLE COMM"),
                    txn_text: "Comcast",
                    txn_date: date(2026, 10, 7),
                    ..input()
                },
                Some(72),
            ),
            (
                "same name chosen by a rule",
                ScoreInput {
                    feed_entity: Some(e),
                    txn_entity: Some(e),
                    txn_date: date(2026, 10, 14),
                    ..input()
                },
                Some(79),
            ),
            (
                "different payee adds nothing",
                ScoreInput {
                    feed_payee: Some("SAFEWAY"),
                    txn_text: "Comcast",
                    ..input()
                },
                Some(70),
            ),
        ];
        for (what, i, want) in cases {
            assert_eq!(score(&i).map(|s| s.0), want, "{what}");
        }
        // Same amount and date alone is strong; three days apart with nothing
        // else is not.
        assert!(score(&input()).unwrap().0 >= STRONG);
        let later = ScoreInput {
            txn_date: date(2026, 10, 7),
            ..input()
        };
        assert!(score(&later).unwrap().0 < STRONG);
    }
}
