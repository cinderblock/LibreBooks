//! Report filters, resolved against the lists (accounts expanded to their
//! subaccounts, names to their jobs…) and applied in two places that must agree:
//! as SQL over the ledger, and in Rust over the facts cash basis synthesises.

use std::collections::HashSet;

use lb_core::{AccountId, ClassId, ClearedStatus, EntityId, ItemId, TxnType};
use rusqlite::types::Value as SqlValue;

use crate::ctx::Ctx;
use crate::ledger::Fact;
use crate::request::{ClearedFilter, ReportFilters};

/// SQL text with positional parameters bound as it is built.
#[derive(Debug, Default)]
pub struct Sql {
    pub text: String,
    pub args: Vec<SqlValue>,
}

impl Sql {
    /// Bind a value; returns its placeholder (`?N`).
    pub fn bind(&mut self, v: impl Into<SqlValue>) -> String {
        self.args.push(v.into());
        format!("?{}", self.args.len())
    }

    /// Bind several text values; returns `(?a, ?b, …)`.
    pub fn bind_list<I: IntoIterator<Item = String>>(&mut self, vs: I) -> String {
        let parts: Vec<String> = vs.into_iter().map(|v| self.bind(v)).collect();
        if parts.is_empty() {
            "(NULL)".to_owned()
        } else {
            format!("({})", parts.join(", "))
        }
    }

    pub fn params(&self) -> rusqlite::ParamsFromIter<std::slice::Iter<'_, SqlValue>> {
        rusqlite::params_from_iter(self.args.iter())
    }
}

/// [`ReportFilters`] with every list expanded to the ids it really means.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub accounts: Option<HashSet<AccountId>>,
    pub names: Option<HashSet<EntityId>>,
    pub classes: Option<HashSet<ClassId>>,
    pub items: Option<HashSet<ItemId>>,
    pub txn_types: Option<HashSet<TxnType>>,
    pub cleared: Option<ClearedFilter>,
    pub amount_min: Option<i64>,
    pub amount_max: Option<i64>,
    /// Lower-cased.
    pub memo: Option<String>,
}

impl Filter {
    pub fn resolve(ctx: &Ctx, f: &ReportFilters) -> Filter {
        let mut accounts: Option<HashSet<AccountId>> = (!f.accounts.is_empty()).then(|| {
            if f.exact {
                f.accounts.iter().copied().collect()
            } else {
                ctx.account_descendants(&f.accounts)
            }
        });
        if !f.account_types.is_empty() {
            let of_type: HashSet<AccountId> = ctx
                .accounts
                .iter()
                .filter(|a| f.account_types.contains(&a.account_type))
                .map(|a| a.id)
                .collect();
            accounts = Some(match accounts {
                Some(a) => a.intersection(&of_type).copied().collect(),
                None => of_type,
            });
        }
        Filter {
            accounts,
            names: (!f.names.is_empty()).then(|| {
                if f.exact {
                    f.names.iter().copied().collect()
                } else {
                    ctx.entity_descendants(&f.names)
                }
            }),
            classes: (!f.classes.is_empty()).then(|| {
                if f.exact {
                    f.classes.iter().copied().collect()
                } else {
                    ctx.class_descendants(&f.classes)
                }
            }),
            items: (!f.items.is_empty()).then(|| {
                if f.exact {
                    f.items.iter().copied().collect()
                } else {
                    ctx.item_descendants(&f.items)
                }
            }),
            txn_types: (!f.txn_types.is_empty()).then(|| f.txn_types.iter().copied().collect()),
            cleared: f.cleared,
            amount_min: f.amount_min.map(|m| m.minor().abs()),
            amount_max: f.amount_max.map(|m| m.minor().abs()),
            memo: f
                .memo
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_lowercase),
        }
    }

    /// Narrow to these accounts (intersecting with any account filter already set).
    #[must_use]
    pub fn and_accounts(mut self, set: HashSet<AccountId>) -> Filter {
        self.accounts = Some(match self.accounts {
            Some(a) => a.intersection(&set).copied().collect(),
            None => set,
        });
        self
    }

    /// `AND …` conditions over the fact columns of alias `f`.
    pub fn sql(&self, q: &mut Sql) -> String {
        let mut out = String::new();
        if let Some(a) = &self.accounts {
            let list = q.bind_list(a.iter().map(ToString::to_string));
            out.push_str(&format!(" AND f.account_id IN {list}"));
        }
        if let Some(n) = &self.names {
            let list = q.bind_list(n.iter().map(ToString::to_string));
            out.push_str(&format!(
                " AND (f.entity_id IN {list} OR f.txn_entity_id IN {list})"
            ));
        }
        if let Some(c) = &self.classes {
            let list = q.bind_list(c.iter().map(ToString::to_string));
            out.push_str(&format!(" AND f.class_id IN {list}"));
        }
        if let Some(i) = &self.items {
            let list = q.bind_list(i.iter().map(ToString::to_string));
            out.push_str(&format!(" AND f.item_id IN {list}"));
        }
        if let Some(t) = &self.txn_types {
            let list = q.bind_list(t.iter().map(|t| t.as_str().to_owned()));
            out.push_str(&format!(" AND f.txn_type IN {list}"));
        }
        match self.cleared {
            Some(ClearedFilter::Cleared) => out.push_str(" AND f.cleared = 'Cleared'"),
            Some(ClearedFilter::NotCleared) => out.push_str(" AND f.cleared <> 'Cleared'"),
            None => {}
        }
        if let Some(m) = self.amount_min {
            let p = q.bind(m);
            out.push_str(&format!(" AND abs(f.txn_amount) >= {p}"));
        }
        if let Some(m) = self.amount_max {
            let p = q.bind(m);
            out.push_str(&format!(" AND abs(f.txn_amount) <= {p}"));
        }
        if let Some(m) = &self.memo {
            // `!` escapes LIKE's wildcards in what the person typed.
            let like = format!(
                "%{}%",
                m.replace('!', "!!").replace('%', "!%").replace('_', "!_")
            );
            let p = q.bind(like);
            out.push_str(&format!(" AND f.memo LIKE {p} ESCAPE '!'"));
        }
        out
    }

    /// The same conditions as [`Filter::sql`], for a fact in memory.
    pub fn matches(&self, f: &Fact) -> bool {
        if let Some(a) = &self.accounts
            && !a.contains(&f.account_id)
        {
            return false;
        }
        if let Some(n) = &self.names {
            let hit = f.entity_id.is_some_and(|e| n.contains(&e))
                || f.txn_entity_id.is_some_and(|e| n.contains(&e));
            if !hit {
                return false;
            }
        }
        if let Some(c) = &self.classes
            && !f.class_id.is_some_and(|x| c.contains(&x))
        {
            return false;
        }
        if let Some(i) = &self.items
            && !f.item_id.is_some_and(|x| i.contains(&x))
        {
            return false;
        }
        if let Some(t) = &self.txn_types
            && !t.contains(&f.txn_type)
        {
            return false;
        }
        match self.cleared {
            Some(ClearedFilter::Cleared) if f.cleared != ClearedStatus::Cleared => return false,
            Some(ClearedFilter::NotCleared) if f.cleared == ClearedStatus::Cleared => {
                return false;
            }
            _ => {}
        }
        let amt = f.txn_amount.minor().abs();
        if self.amount_min.is_some_and(|m| amt < m) || self.amount_max.is_some_and(|m| amt > m) {
            return false;
        }
        if let Some(m) = &self.memo
            && !f
                .memo
                .as_deref()
                .is_some_and(|x| x.to_lowercase().contains(m.as_str()))
        {
            return false;
        }
        true
    }
}
