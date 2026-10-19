//! Importing one QBD record into the books, inside one SQLite transaction.
//!
//! A record is new (create it, or adopt a LibreBooks record that is plainly the
//! same thing: the company's own Accounts Receivable, a name typed in before the
//! first sync), or known through its external reference (update it, carrying
//! the local edit sequence so the store's optimistic concurrency check passes).
//! QBD is the master while LibreBooks is read-only: an update takes QBD's values,
//! keeping only what QBD doesn't have (inventory on hand, local custom fields).

use std::collections::{HashMap, HashSet};

use lb_core::lists::{Class, PaymentMethod, SalesRep, Terms};
use lb_core::{Account, Entity, EntityKind, Item, ItemType, LineId, Money, Txn, TxnId};
use lb_store::extref::{self, ExternalRef};
use lb_store::{Actor, Books, StoreError, Writer, accounts, codec, entities, items, lists, txns};
use serde_json::Value;

use super::resolver::{ResolverState, StoreResolver};
use crate::entity::QbdEntity;
use crate::qbxml::txns::{MissingLink, qbd_line_id};
use crate::qbxml::{self, ListImport, NormaliseError};

/// Why one record didn't import.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Normalise(#[from] NormaliseError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl ImportError {
    /// A deliberate skip (something LibreBooks can't represent) rather than a
    /// failure.
    pub fn is_skip(&self) -> bool {
        matches!(self, ImportError::Normalise(NormaliseError::Unsupported(_)))
    }

    pub fn message(&self) -> String {
        match self {
            ImportError::Store(StoreError::Posting(p)) => {
                format!("LibreBooks' posting rules refused it: {p}")
            }
            other => other.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Created,
    Updated,
}

/// What importing a list record did.
#[derive(Debug)]
pub struct ListOutcome {
    pub outcome: Outcome,
    pub unmapped: Vec<String>,
    pub notes: Vec<String>,
    pub created_lists: Vec<String>,
}

/// What importing a transaction did, and the figures to check.
#[derive(Debug)]
pub struct TxnOutcome {
    pub outcome: Outcome,
    pub txn_id: TxnId,
    pub is_void: bool,
    pub is_posting: bool,
    pub total: Money,
    pub sales_tax: Money,
    pub qbd_total: Option<Money>,
    pub qbd_sales_tax: Option<Money>,
    pub qbd_open_balance: Option<Money>,
    pub missing_links: Vec<MissingLink>,
    pub dropped_credits: usize,
    pub unmapped: Vec<String>,
    pub notes: Vec<String>,
    pub created_lists: Vec<String>,
}

/// Run `f` in one write transaction as the system (imports may write into closed
/// periods: they record what already happened). An error from `f` rolls back
/// everything it did, including lists the resolver created for it.
pub fn write_record<T>(
    books: &Books,
    f: impl FnOnce(&mut Writer<'_>) -> Result<T, ImportError>,
) -> Result<T, ImportError> {
    let mut failure: Option<ImportError> = None;
    let result = books.write(&Actor::system(), |w| match f(w) {
        Ok(v) => Ok(v),
        Err(e) => {
            let message = e.to_string();
            failure = Some(e);
            Err(StoreError::Invalid(message))
        }
    });
    match result {
        Ok(v) => Ok(v),
        Err(e) => Err(failure.unwrap_or(ImportError::Store(e))),
    }
}

/// Is this local record already linked to some *other* QBD record from this
/// source? Then it can't be adopted for this one.
fn linked_elsewhere(
    w: &Writer<'_>,
    source: &str,
    local_kind: &str,
    local_id: &str,
    external_id: &str,
) -> Result<bool, StoreError> {
    Ok(extref::for_local(&w.tx, local_kind, local_id)?
        .iter()
        .any(|r| r.source == source && r.external_id != external_id))
}

fn existing_local<T: std::str::FromStr>(
    existing: Option<&ExternalRef>,
    local_kind: &str,
) -> Option<T> {
    existing
        .filter(|e| e.local_kind == local_kind)
        .and_then(|e| codec::parse_id(&e.local_id).ok())
}

fn qbd_name(row: &Value) -> Option<String> {
    row.get("FullName")
        .or_else(|| row.get("Name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Import one list record. `hint` is the item type the typed item lists gave.
#[allow(clippy::too_many_arguments)]
pub fn import_list(
    w: &mut Writer<'_>,
    state: &mut ResolverState,
    source: &str,
    entity: QbdEntity,
    row: &Value,
    qbd_id: &str,
    edit_sequence: Option<&str>,
    existing: Option<&ExternalRef>,
    hint: Option<ItemType>,
) -> Result<ListOutcome, ImportError> {
    let mut res = StoreResolver::new(w, source, state);
    let (outcome, local_kind, local_id, unmapped, notes) = match entity {
        QbdEntity::Account => {
            let ListImport {
                record: mut a,
                unmapped,
                notes,
            } = qbxml::lists::account(row, &mut res)?;
            let w = &mut *res.w;
            let mut local: Option<Account> = match existing_local(existing, "account") {
                Some(id) => accounts::find(&w.tx, id)?,
                None => None,
            };
            if local.is_none() {
                // Adopt the company's own role account (Accounts Receivable,
                // Undeposited Funds…), or one of the same name and type.
                let by_role = match a.role {
                    Some(role) => accounts::by_role(&w.tx, role)?,
                    None => None,
                };
                let candidate = match by_role {
                    Some(x) => Some(x),
                    None => qbd_name(row)
                        .map(|n| accounts::by_full_name(&w.tx, &n))
                        .transpose()?
                        .flatten()
                        .filter(|x| x.account_type == a.account_type),
                };
                if let Some(c) = candidate
                    && !linked_elsewhere(w, source, "account", &c.id.to_string(), qbd_id)?
                {
                    local = Some(c);
                }
            }
            let outcome = if let Some(l) = &local {
                a.id = l.id;
                a.edit_seq = l.edit_seq;
                a.role = a.role.or(l.role);
                Outcome::Updated
            } else {
                Outcome::Created
            };
            let saved = accounts::save(w, a)?;
            (outcome, "account", saved.id.to_string(), unmapped, notes)
        }
        QbdEntity::Customer | QbdEntity::Vendor | QbdEntity::Employee | QbdEntity::OtherName => {
            let kind = match entity {
                QbdEntity::Customer => EntityKind::Customer,
                QbdEntity::Vendor => EntityKind::Vendor,
                QbdEntity::Employee => EntityKind::Employee,
                _ => EntityKind::OtherName,
            };
            let ListImport {
                record: mut e,
                unmapped,
                notes,
            } = qbxml::lists::entity(kind, row, &mut res)?;
            let w = &mut *res.w;
            let mut local: Option<Entity> = match existing_local(existing, "entity") {
                Some(id) => entities::find(&w.tx, id)?,
                None => None,
            };
            if local.is_none()
                && let Some(c) = qbd_name(row)
                    .map(|n| entities::by_full_name(&w.tx, &n))
                    .transpose()?
                    .flatten()
                    .filter(|x| x.kind == kind)
                && !linked_elsewhere(w, source, "entity", &c.id.to_string(), qbd_id)?
            {
                local = Some(c);
            }
            let outcome = if let Some(l) = &local {
                e.id = l.id;
                e.edit_seq = l.edit_seq;
                let mut fields = l.custom_fields.clone();
                fields.extend(std::mem::take(&mut e.custom_fields));
                e.custom_fields = fields;
                Outcome::Updated
            } else {
                Outcome::Created
            };
            let saved = entities::save(w, e)?;
            (outcome, "entity", saved.id.to_string(), unmapped, notes)
        }
        QbdEntity::Item => {
            let item_type = qbxml::lists::classify_item(row, hint);
            let ListImport {
                record: mut it,
                unmapped,
                mut notes,
            } = qbxml::lists::item(item_type, row, &mut res)?;
            if hint.is_none() && matches!(item_type, ItemType::OtherCharge | ItemType::Subtotal) {
                notes.push(format!(
                    "the bridge doesn't say an item's type; this one was taken to be {} from its fields",
                    item_type.label()
                ));
            }
            let w = &mut *res.w;
            let mut local: Option<Item> = match existing_local(existing, "item") {
                Some(id) => items::find(&w.tx, id)?,
                None => None,
            };
            if local.is_none()
                && let Some(c) = qbd_name(row)
                    .map(|n| items::by_full_name(&w.tx, &n))
                    .transpose()?
                    .flatten()
                    .filter(|x| x.item_type == item_type)
                && !linked_elsewhere(w, source, "item", &c.id.to_string(), qbd_id)?
            {
                local = Some(c);
            }
            let outcome = if let Some(l) = &local {
                it.id = l.id;
                it.edit_seq = l.edit_seq;
                Outcome::Updated
            } else {
                Outcome::Created
            };
            let saved = items::save(w, it)?;
            (outcome, "item", saved.id.to_string(), unmapped, notes)
        }
        QbdEntity::Class => {
            let imp = qbxml::lists::class(row, &mut res)?;
            let w = &mut *res.w;
            let (outcome, id) =
                save_small_list::<Class>(w, source, qbd_id, existing, imp.record, |c, l| {
                    c.id = l.id;
                    c.edit_seq = l.edit_seq;
                })?;
            (outcome, "list", id, imp.unmapped, imp.notes)
        }
        QbdEntity::Terms => {
            let imp = qbxml::lists::terms(row)?;
            let w = &mut *res.w;
            let (outcome, id) =
                save_small_list::<Terms>(w, source, qbd_id, existing, imp.record, |t, l| {
                    t.id = l.id;
                    t.edit_seq = l.edit_seq;
                })?;
            (outcome, "list", id, imp.unmapped, imp.notes)
        }
        QbdEntity::PaymentMethod => {
            let imp = qbxml::lists::payment_method(row)?;
            let w = &mut *res.w;
            let (outcome, id) = save_small_list::<PaymentMethod>(
                w,
                source,
                qbd_id,
                existing,
                imp.record,
                |p, l| {
                    p.id = l.id;
                    p.edit_seq = l.edit_seq;
                },
            )?;
            (outcome, "list", id, imp.unmapped, imp.notes)
        }
        QbdEntity::SalesRep => {
            let imp = qbxml::lists::sales_rep(row, &mut res)?;
            let w = &mut *res.w;
            let (outcome, id) =
                save_small_list::<SalesRep>(w, source, qbd_id, existing, imp.record, |s, l| {
                    s.id = l.id;
                    s.edit_seq = l.edit_seq;
                })?;
            (outcome, "list", id, imp.unmapped, imp.notes)
        }
        other => {
            return Err(NormaliseError::Unsupported(format!(
                "{} isn't imported as a list",
                other.name()
            ))
            .into());
        }
    };
    let created_lists = std::mem::take(&mut res.created_here);
    extref::put(
        res.w,
        source,
        entity.name(),
        qbd_id,
        local_kind,
        &local_id,
        edit_sequence,
        None,
    )?;
    Ok(ListOutcome {
        outcome,
        unmapped,
        notes,
        created_lists,
    })
}

/// Classes, terms, payment methods and sales reps: find the local record by
/// external reference or by name, carry its id and edit sequence, save.
fn save_small_list<T: lists::ListRecord>(
    w: &mut Writer<'_>,
    source: &str,
    qbd_id: &str,
    existing: Option<&ExternalRef>,
    mut rec: T,
    carry: impl FnOnce(&mut T, &T),
) -> Result<(Outcome, String), ImportError> {
    let mut local: Option<T> = match existing.filter(|e| e.local_kind == "list") {
        Some(e) => lists::find(&w.tx, &e.local_id)?,
        None => None,
    };
    if local.is_none()
        && let Some(c) = lists::by_name::<T>(&w.tx, &rec.display_name())?
        && !linked_elsewhere(w, source, "list", &c.id_string(), qbd_id)?
    {
        local = Some(c);
    }
    let outcome = match &local {
        Some(l) => {
            carry(&mut rec, l);
            Outcome::Updated
        }
        None => Outcome::Created,
    };
    let saved = lists::save(w, rec)?;
    Ok((outcome, saved.id_string()))
}

/// Prefix on a stored edit sequence for a transaction imported with a link
/// missing: it never equals QBD's, so the next sync imports it again.
pub const PARTIAL_PREFIX: &str = "partial:";

/// Import one complete transaction.
#[allow(clippy::too_many_arguments)]
pub fn import_txn(
    w: &mut Writer<'_>,
    state: &mut ResolverState,
    source: &str,
    entity: QbdEntity,
    detail: &Value,
    qbd_id: &str,
    edit_sequence: Option<&str>,
    existing: Option<&ExternalRef>,
) -> Result<TxnOutcome, ImportError> {
    let (imp, created_lists) = {
        let mut res = StoreResolver::new(w, source, state);
        let imp = qbxml::txns::txn(entity, detail, &mut res)?;
        (imp, std::mem::take(&mut res.created_here))
    };
    let qbxml::txns::TxnImport {
        mut txn,
        qbd_total,
        qbd_sales_tax,
        qbd_open_balance,
        missing_links,
        unmapped,
        notes,
    } = imp;
    let local: Option<Txn> = match existing_local(existing, "txn") {
        Some(id) => txns::find(&w.tx, id)?,
        None => None,
    };
    let outcome = match &local {
        Some(l) => {
            txn.id = l.id;
            txn.edit_seq = l.edit_seq;
            // Keep LibreBooks' line ids for lines QBD still has, so cleared marks
            // and deposit links stay attached.
            let ids: HashMap<&str, LineId> = l
                .lines
                .iter()
                .filter_map(|line| qbd_line_id(line).map(|q| (q, line.id)))
                .collect();
            for line in &mut txn.lines {
                if let Some(id) = qbd_line_id(line).and_then(|q| ids.get(q)) {
                    line.id = *id;
                }
            }
            Outcome::Updated
        }
        None => Outcome::Created,
    };
    let dropped_credits = dedupe_credits(w, &mut txn)?;
    let is_posting = txn.txn_type.is_posting();
    let saved = txns::save(w, txn)?;
    let partial = missing_links.iter().any(|l| l.exposed);
    let stored_sequence = edit_sequence.map(|es| {
        if partial {
            format!("{PARTIAL_PREFIX}{es}")
        } else {
            es.to_owned()
        }
    });
    extref::put(
        w,
        source,
        entity.name(),
        qbd_id,
        "txn",
        &saved.txn.id.to_string(),
        stored_sequence.as_deref(),
        None,
    )?;
    Ok(TxnOutcome {
        outcome,
        txn_id: saved.txn.id,
        is_void: saved.txn.is_void,
        is_posting,
        total: saved.total,
        sales_tax: saved.sales_tax,
        qbd_total,
        qbd_sales_tax,
        qbd_open_balance,
        missing_links,
        dropped_credits,
        unmapped,
        notes,
        created_lists,
    })
}

/// Credit-only applications (a credit memo used against an invoice) come from
/// the target's `LinkedTxn`, which every payment to that target repeats. Keep
/// each (credit, target) pair once: drop it here if another transaction already
/// records it, or if this one lists it twice. Returns how many were dropped.
fn dedupe_credits(w: &Writer<'_>, txn: &mut Txn) -> Result<usize, StoreError> {
    let before = txn.applications.len();
    let mut seen: HashSet<(TxnId, TxnId)> = HashSet::new();
    let mut keep = Vec::with_capacity(before);
    for a in std::mem::take(&mut txn.applications) {
        let Some(credit) = a
            .credit_txn_id
            .filter(|_| a.amount.is_zero() && a.discount.is_zero())
        else {
            keep.push(a);
            continue;
        };
        if !seen.insert((credit, a.target_txn_id)) {
            continue;
        }
        let elsewhere: bool = w.tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM application WHERE credit_txn_id = ?1 AND target_txn_id = ?2 AND txn_id <> ?3)",
            [credit.to_string(), a.target_txn_id.to_string(), txn.id.to_string()],
            |r| r.get(0),
        )?;
        if !elsewhere {
            keep.push(a);
        }
    }
    txn.applications = keep;
    Ok(before - txn.applications.len())
}

/// What is still open on a transaction, as the store computed it.
pub fn open_balance(books: &Books, id: TxnId) -> Result<Option<Money>, StoreError> {
    books.read(|c| {
        Ok(c.query_row(
            "SELECT open_balance FROM txn WHERE id = ?1",
            [id.to_string()],
            |r| r.get::<_, Option<i64>>(0),
        )?
        .map(Money))
    })
}
