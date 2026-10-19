//! Amazon orders, to explain Amazon card charges.
//!
//! Amazon isn't a money account; a card statement just says `AMZN Mktp US*2K4AB1CD2
//! $42.17`. What was bought is in Amazon's order history, which a person can
//! download through "Request your data" → Your Orders (`Retail.OrderHistory.1.csv`,
//! one row per item). [`parse_order_history`] reads that file;
//! [`match_charges`] pairs card charges with orders; [`annotate`] writes the
//! order onto the downloaded transaction's memo (and keeps the order in its
//! `raw`), so when it's added the register says what the money bought.
//!
//! Amazon charges per **shipment**, not per order, so a charge is compared with
//! each order's total and with each shipment's (the items of an order that
//! shipped on the same date). The design, and the transaction-history scrape
//! that gives exact charge-to-order pairs, are in `plans/feeds.md`.

use std::collections::{BTreeMap, HashSet};

use jiff::civil::Date;
use lb_core::Money;
use lb_store::Writer;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{FeedError, Result};
use crate::ids::FeedTxnId;
use crate::queue::{self, FeedStatus, FeedTxn};
use crate::text::{self, DateOrder};

/// One item row of an order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AmazonItem {
    pub name: String,
    pub asin: Option<String>,
    pub quantity: u32,
    /// What was charged for this row: price, tax and shipping, less discounts.
    pub total: Money,
    pub ship_date: Option<Date>,
}

/// Items of one order that shipped together (and so were charged together).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AmazonShipment {
    pub ship_date: Option<Date>,
    pub total: Money,
    /// Indexes into the order's items.
    pub items: Vec<u32>,
}

/// An order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AmazonOrder {
    pub order_id: String,
    pub order_date: Date,
    pub items: Vec<AmazonItem>,
    pub shipments: Vec<AmazonShipment>,
    pub total: Money,
    /// "Visa - 1234" and the like, when the export says.
    pub payment: Option<String>,
}

impl AmazonOrder {
    /// The order's page on amazon.com.
    pub fn link(&self) -> String {
        format!(
            "https://www.amazon.com/gp/your-account/order-details?orderID={}",
            self.order_id
        )
    }

    /// `Amazon 111-2223334-4445556: Product A; Product B`, short enough for a
    /// memo.
    pub fn memo(&self, items: Option<&[u32]>) -> String {
        let names: Vec<&str> = match items {
            Some(idx) => idx
                .iter()
                .filter_map(|&i| self.items.get(i as usize))
                .map(|i| i.name.as_str())
                .collect(),
            None => self.items.iter().map(|i| i.name.as_str()).collect(),
        };
        let mut memo = format!("Amazon {}: {}", self.order_id, names.join("; "));
        if memo.chars().count() > 200 {
            memo = memo.chars().take(199).collect::<String>() + "…";
        }
        memo
    }
}

/// Read Amazon's order history export (`Retail.OrderHistory.*.csv`). Cancelled
/// rows are left out.
pub fn parse_order_history(bytes: &[u8]) -> Result<Vec<AmazonOrder>> {
    let text = text::decode(bytes);
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| FeedError::parse("Amazon order history", e.to_string()))?
        .iter()
        .map(|h| h.trim().trim_start_matches('\u{feff}').to_lowercase())
        .collect();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let need = |name: &str| {
        col(name).ok_or_else(|| {
            FeedError::parse(
                "Amazon order history",
                format!("there is no \"{name}\" column; is this Retail.OrderHistory.csv?"),
            )
        })
    };
    let (order_col, date_col, total_col) =
        (need("order id")?, need("order date")?, need("total owed")?);
    let name_col = col("product name");
    let asin_col = col("asin");
    let qty_col = col("quantity");
    let ship_col = col("ship date");
    let pay_col = col("payment instrument type");
    let status_col = col("order status");

    let mut orders: BTreeMap<String, AmazonOrder> = BTreeMap::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| FeedError::parse("Amazon order history", e.to_string()))?;
        let get = |i: Option<usize>| {
            i.and_then(|i| rec.get(i))
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "Not Available")
        };
        let Some(order_id) = get(Some(order_col)) else {
            continue;
        };
        if get(status_col).is_some_and(|s| s.eq_ignore_ascii_case("cancelled")) {
            continue;
        }
        let Some(order_date) =
            get(Some(date_col)).and_then(|d| text::parse_date(d, DateOrder::Mdy))
        else {
            continue;
        };
        let Some(total) = get(Some(total_col)).and_then(|t| text::parse_amount(t).ok()) else {
            continue;
        };
        let item = AmazonItem {
            name: get(name_col).unwrap_or("(item)").to_owned(),
            asin: get(asin_col).map(str::to_owned),
            quantity: get(qty_col).and_then(|q| q.parse().ok()).unwrap_or(1),
            total,
            ship_date: get(ship_col).and_then(|d| text::parse_date(d, DateOrder::Mdy)),
        };
        let o = orders
            .entry(order_id.to_owned())
            .or_insert_with(|| AmazonOrder {
                order_id: order_id.to_owned(),
                order_date,
                items: Vec::new(),
                shipments: Vec::new(),
                total: Money::ZERO,
                payment: get(pay_col).map(str::to_owned),
            });
        o.total += item.total;
        o.items.push(item);
    }
    let mut out: Vec<AmazonOrder> = orders.into_values().collect();
    for o in &mut out {
        let mut by_ship: BTreeMap<Option<Date>, AmazonShipment> = BTreeMap::new();
        for (i, item) in o.items.iter().enumerate() {
            let s = by_ship
                .entry(item.ship_date)
                .or_insert_with(|| AmazonShipment {
                    ship_date: item.ship_date,
                    total: Money::ZERO,
                    items: Vec::new(),
                });
            s.total += item.total;
            s.items.push(i as u32);
        }
        o.shipments = by_ship.into_values().collect();
    }
    out.sort_by(|a, b| {
        a.order_date
            .cmp(&b.order_date)
            .then(a.order_id.cmp(&b.order_id))
    });
    Ok(out)
}

/// Does a card payee look like Amazon?
pub fn looks_like_amazon(payee: &str) -> bool {
    let p = payee.to_uppercase();
    // Amazon Web Services bills aren't orders.
    ["AMAZON", "AMZN", "AMZ*", "PRIME VIDEO", "AUDIBLE"]
        .iter()
        .any(|w| p.contains(w))
        && !p.contains("AMAZON WEB SERVICES")
        && !p.contains("AWS.AMAZON")
}

/// A card charge paired with an order (or one shipment of it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AmazonMatch {
    pub feed_txn_id: FeedTxnId,
    pub order_id: String,
    /// `None` when the charge was the whole order.
    pub ship_date: Option<Date>,
    pub memo: String,
    pub link: String,
}

/// Pair Amazon card charges (waiting, money out, Amazon-looking payee) with
/// orders: the same amount as the order or one of its shipments, dated from 2
/// days before to 10 days after the order (or shipment) date, closest first. An
/// order (or shipment) is used once; when a whole order is used, its shipments
/// are too.
pub fn match_charges(charges: &[FeedTxn], orders: &[AmazonOrder]) -> Vec<AmazonMatch> {
    let mut used: HashSet<(usize, Option<usize>)> = HashSet::new();
    let mut charges: Vec<&FeedTxn> = charges
        .iter()
        .filter(|f| {
            f.status == FeedStatus::New
                && f.amount.is_negative()
                && f.payee.as_deref().is_some_and(looks_like_amazon)
        })
        .collect();
    charges.sort_by_key(|f| (f.posted, f.id));
    let mut out = Vec::new();
    for f in charges {
        let amount = f.amount.abs();
        let mut best: Option<(i64, usize, Option<usize>)> = None;
        for (oi, o) in orders.iter().enumerate() {
            if used.contains(&(oi, None)) {
                continue;
            }
            let mut consider = |si: Option<usize>, total: Money, reference: Date| {
                if total != amount || used.contains(&(oi, si)) {
                    return;
                }
                let d = i64::from((f.posted - reference).get_days());
                if !(-2..=10).contains(&d) {
                    return;
                }
                if best.is_none_or(|(bd, ..)| d.abs() < bd) {
                    best = Some((d.abs(), oi, si));
                }
            };
            consider(None, o.total, o.order_date);
            if o.shipments.len() > 1 {
                for (si, s) in o.shipments.iter().enumerate() {
                    consider(Some(si), s.total, s.ship_date.unwrap_or(o.order_date));
                }
            }
        }
        if let Some((_, oi, si)) = best {
            used.insert((oi, si));
            let o = &orders[oi];
            if si.is_none() {
                for s in 0..o.shipments.len() {
                    used.insert((oi, Some(s)));
                }
            }
            let shipment = si.map(|s| &o.shipments[s]);
            out.push(AmazonMatch {
                feed_txn_id: f.id,
                order_id: o.order_id.clone(),
                ship_date: shipment.and_then(|s| s.ship_date),
                memo: o.memo(shipment.map(|s| s.items.as_slice())),
                link: o.link(),
            });
        }
    }
    out
}

/// Write matched orders onto their downloaded transactions: the memo, and the
/// order itself under `raw.amazon`. Returns how many were written.
pub fn annotate(
    w: &mut Writer<'_>,
    matches: &[AmazonMatch],
    orders: &[AmazonOrder],
) -> Result<u32> {
    let mut n = 0;
    for m in matches {
        let order = orders.iter().find(|o| o.order_id == m.order_id);
        let extra = serde_json::json!({
            "order_id": m.order_id,
            "ship_date": m.ship_date,
            "link": m.link,
            "order": order,
        });
        queue::set_memo(w, m.feed_txn_id, &m.memo, Some(("amazon", extra)))?;
        n += 1;
    }
    Ok(n)
}
