//! Company information, the closing date, and the audit trail.

use axum::Json;
use axum::extract::{Path, Query, State};
use lb_core::{CompanyId, CompanyInfo};
use lb_store::{audit, company};
use serde::Deserialize;
use serde_json::{Value, json};

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::ApiResult;
use crate::state::AppState;

pub async fn get_info(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<CompanyId>,
) -> ApiResult<Json<CompanyInfo>> {
    let books = s.books_for(&p, id, Role::Viewer)?;
    Ok(Json(blocking(move || books.read(company::info)).await?))
}

pub async fn put_info(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<CompanyId>,
    Json(info): Json<CompanyInfo>,
) -> ApiResult<Json<CompanyInfo>> {
    let books = s.books_for(&p, id, Role::Admin)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || {
            books.write(&actor, |w| company::set_info(w, &info))?;
            books.read(company::info)
        })
        .await?,
    ))
}

pub async fn get_closing_date(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<CompanyId>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, id, Role::Viewer)?;
    let d = blocking(move || books.read(company::closing_date)).await?;
    Ok(Json(json!({ "closing_date": d })))
}

#[derive(Deserialize)]
pub struct ClosingDate {
    closing_date: Option<jiff::civil::Date>,
}

pub async fn put_closing_date(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<CompanyId>,
    Json(body): Json<ClosingDate>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, id, Role::Admin)?;
    let actor = p.actor();
    let d = body.closing_date;
    blocking(move || books.write(&actor, |w| company::set_closing_date(w, d))).await?;
    Ok(Json(json!({ "closing_date": d })))
}

#[derive(Deserialize)]
pub struct AuditQuery {
    record_kind: Option<String>,
    record_id: Option<String>,
    before_id: Option<i64>,
    limit: Option<u32>,
}

pub async fn audit(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<CompanyId>,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Vec<audit::AuditEntry>>> {
    let books = s.books_for(&p, id, Role::Bookkeeper)?;
    Ok(Json(
        blocking(move || {
            books.read(|c| match (q.record_kind, q.record_id) {
                (Some(k), Some(r)) => audit::history(c, &k, &r),
                _ => audit::recent(c, q.limit.unwrap_or(200).min(2000), q.before_id),
            })
        })
        .await?,
    ))
}
