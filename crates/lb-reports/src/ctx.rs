//! Everything a report needs to know about the company besides the ledger: the
//! chart of accounts with its hierarchy, names, items, classes, terms and
//! preferences. Loaded once per report run, inside the same read transaction as
//! the ledger queries, so names and numbers are one consistent snapshot.

use std::collections::{HashMap, HashSet};

use jiff::civil::Date;
use lb_core::lists::{Class, Terms};
use lb_core::{
    Account, AccountId, AccountRole, AccountType, ClassId, CompanyInfo, Entity, EntityId,
    EntityKind, Item, ItemId, ItemType, TermsId,
};
use lb_store::{accounts, company, entities, items, lists};
use rusqlite::Connection;

use crate::Result;

/// A company as reports see it.
#[derive(Debug)]
pub struct Ctx {
    pub info: CompanyInfo,
    pub today: Date,
    /// Chart order (type, number, name).
    pub accounts: Vec<Account>,
    account_ix: HashMap<AccountId, usize>,
    children: HashMap<AccountId, Vec<AccountId>>,
    pub roles: HashMap<AccountRole, AccountId>,
    pub entities: HashMap<EntityId, Entity>,
    entity_children: HashMap<EntityId, Vec<EntityId>>,
    pub items: HashMap<ItemId, Item>,
    item_children: HashMap<ItemId, Vec<ItemId>>,
    pub classes: HashMap<ClassId, Class>,
    class_children: HashMap<ClassId, Vec<ClassId>>,
    pub terms: HashMap<TermsId, Terms>,
}

impl Ctx {
    pub fn load(c: &Connection, today: Date) -> Result<Ctx> {
        let info = company::info(c)?;
        let accounts = accounts::list(c, true)?;
        let account_ix = accounts
            .iter()
            .enumerate()
            .map(|(i, a)| (a.id, i))
            .collect();
        let mut children: HashMap<AccountId, Vec<AccountId>> = HashMap::new();
        let mut roles = HashMap::new();
        for a in &accounts {
            if let Some(p) = a.parent_id {
                children.entry(p).or_default().push(a.id);
            }
            if let Some(r) = a.role {
                roles.insert(r, a.id);
            }
        }
        let mut entity_children: HashMap<EntityId, Vec<EntityId>> = HashMap::new();
        let ents = entities::list(c, None, true)?;
        for e in &ents {
            if let Some(p) = e.parent_id {
                entity_children.entry(p).or_default().push(e.id);
            }
        }
        let mut item_children: HashMap<ItemId, Vec<ItemId>> = HashMap::new();
        let its = items::list(c, None, true)?;
        for i in &its {
            if let Some(p) = i.parent_id {
                item_children.entry(p).or_default().push(i.id);
            }
        }
        let mut class_children: HashMap<ClassId, Vec<ClassId>> = HashMap::new();
        let cls: Vec<Class> = lists::list(c, true)?;
        for cl in &cls {
            if let Some(p) = cl.parent_id {
                class_children.entry(p).or_default().push(cl.id);
            }
        }
        let terms: Vec<Terms> = lists::list(c, true)?;
        Ok(Ctx {
            info,
            today,
            accounts,
            account_ix,
            children,
            roles,
            entities: ents.into_iter().map(|e| (e.id, e)).collect(),
            entity_children,
            items: its.into_iter().map(|i| (i.id, i)).collect(),
            item_children,
            classes: cls.into_iter().map(|c| (c.id, c)).collect(),
            class_children,
            terms: terms.into_iter().map(|t| (t.id, t)).collect(),
        })
    }

    pub fn fiscal_start(&self) -> i8 {
        self.info.fiscal_year_start_month
    }

    pub fn account(&self, id: AccountId) -> Option<&Account> {
        self.account_ix.get(&id).map(|&i| &self.accounts[i])
    }

    pub fn account_type(&self, id: AccountId) -> Option<AccountType> {
        self.account(id).map(|a| a.account_type)
    }

    /// Chart position, for sorting rows the way the chart lists them.
    pub fn account_order(&self, id: AccountId) -> usize {
        self.account_ix.get(&id).copied().unwrap_or(usize::MAX)
    }

    pub fn account_children(&self, id: AccountId) -> &[AccountId] {
        self.children.get(&id).map_or(&[], Vec::as_slice)
    }

    /// An account's name as a report row shows it: its own name (the indent shows
    /// its parents), with its number first when numbering is on ("6000 · Rent").
    pub fn account_label(&self, id: AccountId) -> String {
        match self.account(id) {
            Some(a) => match (&a.number, self.info.preferences.use_account_numbers) {
                (Some(n), true) => format!("{n} · {}", a.name),
                _ => a.name.clone(),
            },
            None => "(unknown account)".to_owned(),
        }
    }

    /// An account's full name (`Parent:Child`), with its number when numbering is on.
    pub fn account_full_label(&self, id: AccountId) -> String {
        match self.account(id) {
            Some(a) => match (&a.number, self.info.preferences.use_account_numbers) {
                (Some(n), true) => format!("{n} · {}", a.full_name),
                _ => a.full_name.clone(),
            },
            None => "(unknown account)".to_owned(),
        }
    }

    pub fn role(&self, role: AccountRole) -> Option<AccountId> {
        self.roles.get(&role).copied()
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn entity_kind(&self, id: EntityId) -> Option<EntityKind> {
        self.entity(id).map(|e| e.kind)
    }

    pub fn entity_name(&self, id: Option<EntityId>) -> Option<String> {
        id.and_then(|id| self.entity(id))
            .map(|e| e.full_name.clone())
    }

    pub fn entity_children(&self, id: EntityId) -> &[EntityId] {
        self.entity_children.get(&id).map_or(&[], Vec::as_slice)
    }

    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items.get(&id)
    }

    pub fn item_is_inventory(&self, id: Option<ItemId>) -> bool {
        id.and_then(|i| self.item(i))
            .is_some_and(|i| i.item_type.is_inventory())
    }

    pub fn item_type(&self, id: ItemId) -> Option<ItemType> {
        self.item(id).map(|i| i.item_type)
    }

    pub fn class_name(&self, id: Option<ClassId>) -> Option<String> {
        id.and_then(|c| self.classes.get(&c))
            .map(|c| c.full_name.clone())
    }

    /// `ids` with every subaccount below them.
    pub fn account_descendants(&self, ids: &[AccountId]) -> HashSet<AccountId> {
        descend(ids, |a| self.account_children(a))
    }

    pub fn entity_descendants(&self, ids: &[EntityId]) -> HashSet<EntityId> {
        descend(ids, |e| self.entity_children(e))
    }

    pub fn item_descendants(&self, ids: &[ItemId]) -> HashSet<ItemId> {
        descend(ids, |i| {
            self.item_children
                .get(&i)
                .map_or(&[] as &[ItemId], Vec::as_slice)
        })
    }

    pub fn class_descendants(&self, ids: &[ClassId]) -> HashSet<ClassId> {
        descend(ids, |c| {
            self.class_children
                .get(&c)
                .map_or(&[] as &[ClassId], Vec::as_slice)
        })
    }

    /// The customer or job a line belongs to, for job costing: the line's own name
    /// when it is a customer (an expense passed on to a job), else the
    /// transaction's (an invoice's customer).
    pub fn job_of(
        &self,
        entity: Option<EntityId>,
        txn_entity: Option<EntityId>,
    ) -> Option<EntityId> {
        [entity, txn_entity]
            .into_iter()
            .flatten()
            .find(|e| self.entity_kind(*e) == Some(EntityKind::Customer))
    }

    /// The vendor a line belongs to: the transaction's payee when it is a vendor,
    /// else the line's own name.
    pub fn vendor_of(
        &self,
        entity: Option<EntityId>,
        txn_entity: Option<EntityId>,
    ) -> Option<EntityId> {
        [txn_entity, entity]
            .into_iter()
            .flatten()
            .find(|e| self.entity_kind(*e) == Some(EntityKind::Vendor))
    }
}

fn descend<'a, T: Copy + Eq + std::hash::Hash + 'a>(
    roots: &[T],
    children: impl Fn(T) -> &'a [T],
) -> HashSet<T> {
    let mut out = HashSet::new();
    let mut stack: Vec<T> = roots.to_vec();
    while let Some(x) = stack.pop() {
        if out.insert(x) {
            stack.extend_from_slice(children(x));
        }
    }
    out
}
