//! The posting rules' view of a company file.

use lb_core::{
    AccountId, AccountInfo, AccountRole, AccountType, EntityId, EntityKind, Item, ItemId,
    OpenTxnInfo, PostingContext, SalesTaxCodeId, TxnId, TxnType,
};
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;

use crate::codec::{self, SqlDate};

/// Answers [`PostingContext`] questions from the database, inside whatever
/// transaction the caller holds.
#[derive(Debug)]
pub struct StoreContext<'c> {
    pub conn: &'c Connection,
}

impl<'c> StoreContext<'c> {
    pub fn new(conn: &'c Connection) -> Self {
        StoreContext { conn }
    }
}

impl PostingContext for StoreContext<'_> {
    fn account(&self, id: AccountId) -> Option<AccountInfo> {
        self.conn
            .query_row(
                "SELECT account_type, is_active FROM account WHERE id = ?1",
                [id.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|(t, is_active)| {
                AccountType::from_qbxml(&t).map(|account_type| AccountInfo {
                    account_type,
                    is_active,
                })
            })
    }

    fn role_account(&self, role: AccountRole) -> Option<AccountId> {
        self.conn
            .query_row(
                "SELECT id FROM account WHERE role = ?1",
                [role.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|s| codec::parse_id(&s).ok())
    }

    fn item(&self, id: ItemId) -> Option<Item> {
        crate::items::find(self.conn, id).ok().flatten()
    }

    fn entity_kind(&self, id: EntityId) -> Option<EntityKind> {
        self.conn
            .query_row(
                "SELECT kind FROM entity WHERE id = ?1",
                [id.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|k| EntityKind::parse(&k))
    }

    fn tax_code_is_taxable(&self, id: SalesTaxCodeId) -> bool {
        self.conn
            .query_row(
                "SELECT json_extract(doc, '$.is_taxable') FROM list_entry WHERE list = 'sales_tax_code' AND id = ?1",
                [id.to_string()],
                |r| r.get::<_, Option<bool>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten()
            .unwrap_or(false)
    }

    fn open_txn(&self, id: TxnId) -> Option<OpenTxnInfo> {
        self.conn
            .query_row(
                "SELECT txn_type, entity_id, ar_ap_account_id FROM txn WHERE id = ?1",
                [id.to_string()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|(t, e, a)| {
                Some(OpenTxnInfo {
                    txn_type: TxnType::parse(&t)?,
                    entity_id: e.and_then(|s| codec::parse_id(&s).ok()),
                    ar_ap_account_id: a.and_then(|s| codec::parse_id(&s).ok()),
                })
            })
    }

    fn average_cost(&self, item: ItemId, date: jiff::civil::Date, excluding: TxnId) -> Decimal {
        average_cost(self.conn, item, date, Some(excluding)).unwrap_or(Decimal::ZERO)
    }
}

/// Average cost per unit of an inventory item from its asset postings on or before
/// `date`: total value over total quantity.
pub fn average_cost(
    c: &Connection,
    item: ItemId,
    date: jiff::civil::Date,
    excluding: Option<TxnId>,
) -> crate::Result<Decimal> {
    let (qty, value) = on_hand(c, item, date, excluding)?;
    if qty <= Decimal::ZERO {
        // Nothing on hand: fall back to the item's purchase cost.
        let item = crate::items::find(c, item)?;
        return Ok(item.and_then(|i| i.purchase_cost).unwrap_or(Decimal::ZERO));
    }
    Ok((value / qty).round_dp(5))
}

/// Quantity on hand and its value (in major units) as of `date`.
pub fn on_hand(
    c: &Connection,
    item: ItemId,
    date: jiff::civil::Date,
    excluding: Option<TxnId>,
) -> crate::Result<(Decimal, Decimal)> {
    let mut stmt = c.prepare(
        "SELECT quantity, amount FROM posting
         WHERE item_id = ?1 AND quantity IS NOT NULL AND date <= ?2 AND (?3 IS NULL OR txn_id <> ?3)",
    )?;
    let rows = stmt.query_map(
        params![
            item.to_string(),
            SqlDate(date),
            excluding.map(|t| t.to_string())
        ],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )?;
    let mut qty = Decimal::ZERO;
    let mut value = Decimal::ZERO;
    for row in rows {
        let (q, a) = row?;
        qty += q.parse::<Decimal>().unwrap_or(Decimal::ZERO);
        value += lb_core::Money(a).to_decimal();
    }
    Ok((qty, value))
}
