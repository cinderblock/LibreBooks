//! Memorized transactions and reports, and other saved documents.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lb_core::{CompanyId, Txn};
use lb_store::saved::{self, Memorized, Saved};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Kinds the API accepts; anything else is a typo.
const KINDS: [&str; 3] = [saved::MEMORIZED_TXN, saved::MEMORIZED_REPORT, "view"];

fn check_kind(kind: &str) -> ApiResult<()> {
    if KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(ApiError::not_found("saved kind"))
    }
}

pub async fn list(
    State(s): State<AppState>,
    p: Principal,
    Path((company, kind)): Path<(CompanyId, String)>,
) -> ApiResult<Json<Vec<Saved>>> {
    check_kind(&kind)?;
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        blocking(move || books.read(|c| saved::list(c, &kind))).await?,
    ))
}

#[derive(Deserialize)]
pub struct PutBody {
    name: String,
    doc: Value,
}

pub async fn create(
    State(s): State<AppState>,
    p: Principal,
    Path((company, kind)): Path<(CompanyId, String)>,
    Json(b): Json<PutBody>,
) -> ApiResult<(StatusCode, Json<Saved>)> {
    check_kind(&kind)?;
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    let out = blocking(move || books.write(&actor, |w| put(w, None, &kind, &b))).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

fn put(
    w: &mut lb_store::Writer<'_>,
    id: Option<Uuid>,
    kind: &str,
    b: &PutBody,
) -> lb_store::Result<Saved> {
    if kind == saved::MEMORIZED_TXN {
        let m: Memorized = serde_json::from_value(b.doc.clone())?;
        saved::memorize(w, id, &b.name, &m)
    } else {
        saved::put(w, id, kind, &b.name, &b.doc)
    }
}

pub async fn update(
    State(s): State<AppState>,
    p: Principal,
    Path((company, kind, id)): Path<(CompanyId, String, Uuid)>,
    Json(b): Json<PutBody>,
) -> ApiResult<Json<Saved>> {
    check_kind(&kind)?;
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || books.write(&actor, |w| put(w, Some(id), &kind, &b))).await?,
    ))
}

pub async fn delete(
    State(s): State<AppState>,
    p: Principal,
    Path((company, kind, id)): Path<(CompanyId, String, Uuid)>,
) -> ApiResult<StatusCode> {
    check_kind(&kind)?;
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    blocking(move || books.write(&actor, |w| saved::delete(w, id))).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn due(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let today = jiff::Zoned::now().date();
    let list = blocking(move || books.read(|c| saved::due(c, today))).await?;
    Ok(Json(json!(
        list.into_iter()
            .map(|(s, m, date)| json!({ "id": s.id, "name": s.name, "due": date, "repeat": m.repeat }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
pub struct UseQuery {
    date: Option<jiff::civil::Date>,
}

/// A new transaction from a memorized one, not saved: the form opens with it.
pub async fn use_memorized(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, Uuid)>,
    Query(q): Query<UseQuery>,
) -> ApiResult<Json<Txn>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let date = q.date.unwrap_or_else(|| jiff::Zoned::now().date());
    let saved = blocking(move || books.read(|c| saved::get(c, id))).await?;
    if saved.kind != saved::MEMORIZED_TXN {
        return Err(ApiError::not_found("memorized transaction"));
    }
    let m: Memorized =
        serde_json::from_value(saved.doc).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(saved::instantiate(&m.template, date)))
}
