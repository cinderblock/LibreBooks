//! Transactions, registers, balances, and the open items payments draw on.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lb_core::{
    AccountId, AccountInfo, AccountRole, ClearedStatus, CompanyId, EntityId, Item, ItemId, Money,
    OpenTxnInfo, PostingContext, SalesTaxCodeId, Txn, TxnId, TxnType, post,
};
use lb_store::txns::{SaveOutcome, TxnFilter, TxnSummary};
use lb_store::{Fulfilment, StoreContext, StoreError, audit, balances, orders, txns};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// `TxnFilter` as a query string (`types` comma-separated).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TxnQuery {
    types: Option<String>,
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
    entity_id: Option<EntityId>,
    include_jobs: bool,
    account_id: Option<AccountId>,
    number: Option<String>,
    text: Option<String>,
    amount: Option<i64>,
    open_only: bool,
    include_void: bool,
    limit: Option<u32>,
    offset: Option<u32>,
    descending: bool,
}

impl TxnQuery {
    fn into_filter(self) -> ApiResult<TxnFilter> {
        let types = match self.types.as_deref() {
            Some(t) if !t.is_empty() => t
                .split(',')
                .map(|s| {
                    TxnType::parse(s.trim())
                        .ok_or_else(|| ApiError::invalid(format!("unknown transaction type {s}")))
                })
                .collect::<ApiResult<Vec<_>>>()?,
            _ => Vec::new(),
        };
        Ok(TxnFilter {
            types,
            from: self.from,
            to: self.to,
            entity_id: self.entity_id,
            include_jobs: self.include_jobs,
            account_id: self.account_id,
            number: self.number,
            text: self.text,
            amount: self.amount.map(Money),
            open_only: self.open_only,
            include_void: self.include_void,
            limit: self.limit,
            offset: self.offset,
            descending: self.descending,
        })
    }
}

pub async fn query(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<TxnQuery>,
) -> ApiResult<Json<Vec<TxnSummary>>> {
    let filter = q.into_filter()?;
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        blocking(move || books.read(|c| txns::query(c, &filter))).await?,
    ))
}

pub async fn get(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, TxnId)>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let (txn, postings) =
        blocking(move || books.read(|c| Ok((txns::get(c, id)?, txns::postings(c, id)?)))).await?;
    Ok(Json(json!({ "txn": txn, "postings": postings })))
}

pub async fn create(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(txn): Json<Txn>,
) -> ApiResult<(StatusCode, Json<SaveOutcome>)> {
    if txn.edit_seq != 0 {
        return Err(ApiError::invalid("a new transaction has edit_seq 0"));
    }
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    let out = blocking(move || books.write(&actor, |w| txns::save(w, txn))).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

pub async fn update(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, TxnId)>,
    Json(txn): Json<Txn>,
) -> ApiResult<Json<SaveOutcome>> {
    if txn.id != id {
        return Err(ApiError::invalid("the id in the path and the body differ"));
    }
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || books.write(&actor, |w| txns::save(w, txn))).await?,
    ))
}

/// Post without saving: live totals and the "show me the postings" view. Role
/// accounts the company doesn't have yet are stood in for, so a first invoice
/// with sales tax still previews.
pub async fn preview(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(txn): Json<Txn>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let posted = blocking(move || {
        books.read(|c| {
            let ctx = PreviewContext {
                inner: StoreContext::new(c),
            };
            post(&txn, &ctx).map_err(StoreError::from)
        })
    })
    .await?;
    Ok(Json(json!({
        "postings": posted.postings,
        "total": posted.total,
        "sales_tax": posted.sales_tax,
        "warnings": Vec::<String>::new(),
    })))
}

/// A posting context that invents stand-ins for missing role accounts.
struct PreviewContext<'c> {
    inner: StoreContext<'c>,
}

/// A fixed id per role for the stand-ins (never stored).
fn stand_in(role: AccountRole) -> AccountId {
    let n = role as u128 + 1;
    AccountId::from_uuid(uuid::Uuid::from_u128(
        0x4c42_0000_0000_7000_8000_0000_0000_0000 | n,
    ))
}

impl PostingContext for PreviewContext<'_> {
    fn account(&self, id: AccountId) -> Option<AccountInfo> {
        self.inner.account(id).or_else(|| {
            ROLES
                .iter()
                .find(|r| stand_in(**r) == id)
                .map(|r| AccountInfo {
                    account_type: r.default_shape().0,
                    is_active: true,
                })
        })
    }
    fn role_account(&self, role: AccountRole) -> Option<AccountId> {
        self.inner.role_account(role).or(Some(stand_in(role)))
    }
    fn item(&self, id: ItemId) -> Option<Item> {
        self.inner.item(id)
    }
    fn entity_kind(&self, id: EntityId) -> Option<lb_core::EntityKind> {
        self.inner.entity_kind(id)
    }
    fn tax_code_is_taxable(&self, id: SalesTaxCodeId) -> bool {
        self.inner.tax_code_is_taxable(id)
    }
    fn open_txn(&self, id: TxnId) -> Option<OpenTxnInfo> {
        self.inner.open_txn(id)
    }
    fn average_cost(&self, item: ItemId, date: jiff::civil::Date, excluding: TxnId) -> Decimal {
        self.inner.average_cost(item, date, excluding)
    }
}

const ROLES: [AccountRole; 15] = [
    AccountRole::AccountsReceivable,
    AccountRole::AccountsPayable,
    AccountRole::UndepositedFunds,
    AccountRole::OpeningBalanceEquity,
    AccountRole::RetainedEarnings,
    AccountRole::SalesTaxPayable,
    AccountRole::InventoryAsset,
    AccountRole::CostOfGoodsSold,
    AccountRole::PayrollLiabilities,
    AccountRole::PayrollExpenses,
    AccountRole::AskMyAccountant,
    AccountRole::ReconciliationDiscrepancies,
    AccountRole::Estimates,
    AccountRole::PurchaseOrders,
    AccountRole::SalesOrders,
];

#[derive(Deserialize)]
pub struct EditSeq {
    edit_seq: i64,
}

pub async fn void(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, TxnId)>,
    Json(body): Json<EditSeq>,
) -> ApiResult<Json<SaveOutcome>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || books.write(&actor, |w| txns::void(w, id, body.edit_seq))).await?,
    ))
}

pub async fn delete(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, TxnId)>,
    Query(q): Query<EditSeq>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    blocking(move || books.write(&actor, |w| txns::delete(w, id, q.edit_seq))).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct NextNumberQuery {
    r#type: String,
    account_id: Option<AccountId>,
}

pub async fn next_number(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<NextNumberQuery>,
) -> ApiResult<Json<Value>> {
    let t =
        TxnType::parse(&q.r#type).ok_or_else(|| ApiError::invalid("unknown transaction type"))?;
    let books = s.books_for(&p, company, Role::Viewer)?;
    let n = blocking(move || books.read(|c| txns::next_number(c, t, q.account_id))).await?;
    Ok(Json(json!({ "number": n })))
}

pub async fn history(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, TxnId)>,
) -> ApiResult<Json<Vec<audit::AuditEntry>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        blocking(move || books.read(|c| audit::history(c, "txn", &id.to_string()))).await?,
    ))
}

/// `GET /txns/:id/fulfilment`: how much of a purchase order, sales order or
/// estimate has been received or invoiced, line by line. 404 for anything that
/// isn't an order.
pub async fn fulfilment(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, TxnId)>,
) -> ApiResult<Json<Fulfilment>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        blocking(move || books.read(|c| orders::fulfilment(c, id))).await?,
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RangeQuery {
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
}

pub async fn register(
    State(s): State<AppState>,
    p: Principal,
    Path((company, account)): Path<(CompanyId, AccountId)>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let (opening, lines) =
        blocking(move || books.read(|c| balances::register(c, account, q.from, q.to))).await?;
    Ok(Json(json!({ "opening": opening, "lines": lines })))
}

#[derive(Deserialize)]
pub struct ClearedBody {
    status: ClearedStatus,
}

pub async fn set_cleared(
    State(s): State<AppState>,
    p: Principal,
    Path((company, account, txn)): Path<(CompanyId, AccountId, TxnId)>,
    Json(body): Json<ClearedBody>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    blocking(move || {
        books.write(&actor, |w| {
            balances::set_cleared(w, txn, account, body.status)
        })
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn balances(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<Vec<lb_store::AccountBalance>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let sums = blocking(move || books.read(|c| balances::sums(c, q.from, q.to))).await?;
    let mut out: Vec<lb_store::AccountBalance> = sums
        .into_iter()
        .map(|(account_id, debit_positive)| lb_store::AccountBalance {
            account_id,
            debit_positive,
        })
        .collect();
    out.sort_by_key(|b| b.account_id);
    Ok(Json(out))
}

/// `GET /balances/names`: every name's A/R and A/P balance (with its jobs
/// rolled up), what the Customer and Vendor Centers show in their lists.
pub async fn name_balances(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Vec<lb_store::NameBalance>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        blocking(move || books.read(balances::name_balances)).await?,
    ))
}

#[derive(Deserialize)]
pub struct OpenQuery {
    entity_id: EntityId,
    kind: String,
}

/// Open items of a name and its jobs: invoices and credits (`receivable`, for
/// Receive Payments), bills and credits (`payable`, for Pay Bills), or the
/// orders still to be received or invoiced (`purchase_orders`, `sales_orders`,
/// `estimates`, for Receive Items and Create Invoice from an order). An order's
/// `open_balance` is what remains on it.
pub async fn open_items(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<OpenQuery>,
) -> ApiResult<Json<Vec<TxnSummary>>> {
    let types = match q.kind.as_str() {
        "receivable" => vec![
            TxnType::Invoice,
            TxnType::StatementCharge,
            TxnType::CreditMemo,
            TxnType::ReceivePayment,
        ],
        "payable" => vec![
            TxnType::Bill,
            TxnType::VendorCredit,
            TxnType::BillPaymentCheck,
            TxnType::BillPaymentCreditCard,
        ],
        "purchase_orders" => vec![TxnType::PurchaseOrder],
        "sales_orders" => vec![TxnType::SalesOrder],
        "estimates" => vec![TxnType::Estimate],
        _ => {
            return Err(ApiError::invalid(
                "kind is receivable, payable, purchase_orders, sales_orders or estimates",
            ));
        }
    };
    let filter = TxnFilter {
        types,
        entity_id: Some(q.entity_id),
        include_jobs: true,
        open_only: true,
        limit: Some(5000),
        ..TxnFilter::default()
    };
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        blocking(move || books.read(|c| txns::query(c, &filter))).await?,
    ))
}

pub async fn undeposited(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Vec<TxnSummary>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(blocking(move || books.read(txns::undeposited)).await?))
}
