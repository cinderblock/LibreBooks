//! Reconciliation, opening balances and attachments.

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use lb_core::{AccountId, AttachmentId, CompanyId, Money, ReconciliationId, TxnId};
use lb_store::accounts;
use lb_store::attachments::{self, AttachmentMeta};
use lb_store::reconcile::{self, Reconciliation, StatementCharge};
use serde::Deserialize;
use serde_json::{Value, json};

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub async fn reconcile_overview(
    State(s): State<AppState>,
    p: Principal,
    Path((company, account)): Path<(CompanyId, AccountId)>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let (current, history, beginning) = blocking(move || {
        books.read(|c| {
            Ok((
                reconcile::in_progress(c, account)?,
                reconcile::history(c, account)?,
                reconcile::beginning_balance(c, account)?,
            ))
        })
    })
    .await?;
    Ok(Json(
        json!({ "in_progress": current, "history": history, "beginning_balance": beginning }),
    ))
}

#[derive(Deserialize)]
pub struct StartBody {
    statement_date: jiff::civil::Date,
    ending_balance: Money,
    service_charge: Option<StatementCharge>,
    interest: Option<StatementCharge>,
}

pub async fn reconcile_start(
    State(s): State<AppState>,
    p: Principal,
    Path((company, account)): Path<(CompanyId, AccountId)>,
    Json(b): Json<StartBody>,
) -> ApiResult<Json<Reconciliation>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || {
            books.write(&actor, |w| {
                reconcile::start(
                    w,
                    account,
                    b.statement_date,
                    b.ending_balance,
                    b.service_charge,
                    b.interest,
                )
            })
        })
        .await?,
    ))
}

pub async fn reconcile_undo(
    State(s): State<AppState>,
    p: Principal,
    Path((company, account)): Path<(CompanyId, AccountId)>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    let undone =
        blocking(move || books.write(&actor, |w| reconcile::undo_last(w, account))).await?;
    Ok(Json(json!({ "undone": undone })))
}

pub async fn reconciliation(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, ReconciliationId)>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let (rec, lines, status) = blocking(move || {
        books.read(|c| {
            Ok((
                reconcile::get(c, id)?,
                reconcile::lines(c, id)?,
                reconcile::status(c, id)?,
            ))
        })
    })
    .await?;
    Ok(Json(
        json!({ "reconciliation": rec, "lines": lines, "status": status }),
    ))
}

#[derive(Deserialize)]
pub struct MarkBody {
    txn_ids: Vec<TxnId>,
    marked: bool,
}

pub async fn reconcile_mark(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, ReconciliationId)>,
    Json(b): Json<MarkBody>,
) -> ApiResult<Json<reconcile::ReconcileStatus>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || {
            books.write(&actor, |w| reconcile::mark(w, id, &b.txn_ids, b.marked))?;
            books.read(|c| reconcile::status(c, id))
        })
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct FinishBody {
    #[serde(default)]
    adjust: bool,
}

pub async fn reconcile_finish(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, ReconciliationId)>,
    Json(b): Json<FinishBody>,
) -> ApiResult<Json<Reconciliation>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || books.write(&actor, |w| reconcile::finish(w, id, b.adjust))).await?,
    ))
}

pub async fn reconcile_discard(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, ReconciliationId)>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    blocking(move || books.write(&actor, |w| reconcile::discard(w, id))).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct OpeningBody {
    amount: Money,
    date: jiff::civil::Date,
}

pub async fn opening_balance(
    State(s): State<AppState>,
    p: Principal,
    Path((company, account)): Path<(CompanyId, AccountId)>,
    Json(b): Json<OpeningBody>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    let txn = blocking(move || {
        books.write(&actor, |w| {
            accounts::set_opening_balance(w, account, b.amount, b.date)
        })
    })
    .await?;
    Ok(Json(json!({ "txn_id": txn })))
}

/// `POST /attachments`: multipart with a `file` part and optional `record_kind` /
/// `record_id` text parts.
pub async fn upload(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    mut form: Multipart,
) -> ApiResult<(StatusCode, Json<AttachmentMeta>)> {
    let mut file: Option<(String, String, Vec<u8>)> = None;
    let mut record_kind: Option<String> = None;
    let mut record_id: Option<String> = None;
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::invalid(e.to_string()))?
    {
        match field.name() {
            Some("file") => {
                let name = field.file_name().unwrap_or("attachment").to_owned();
                let mime = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::invalid(e.to_string()))?;
                file = Some((name, mime, bytes.to_vec()));
            }
            Some("record_kind") => {
                record_kind = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::invalid(e.to_string()))?,
                );
            }
            Some("record_id") => {
                record_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::invalid(e.to_string()))?,
                );
            }
            _ => {}
        }
    }
    let (name, mime, data) = file.ok_or_else(|| ApiError::invalid("no file was sent"))?;
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    let meta = blocking(move || {
        books.write(&actor, |w| {
            let record = record_kind.as_deref().zip(record_id.as_deref());
            attachments::add(w, record, &name, &mime, &data)
        })
    })
    .await?;
    Ok((StatusCode::CREATED, Json(meta)))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct AttachmentQuery {
    record_kind: Option<String>,
    record_id: Option<String>,
    /// Every document, attached or not (the "Attached" tab).
    all: bool,
}

pub async fn list_attachments(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<AttachmentQuery>,
) -> ApiResult<Json<Vec<AttachmentMeta>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        blocking(move || {
            books.read(|c| match (q.all, q.record_kind, q.record_id) {
                (true, _, _) => attachments::all(c),
                (false, Some(k), Some(i)) => attachments::for_record(c, &k, &i),
                _ => attachments::inbox(c),
            })
        })
        .await?,
    ))
}

pub async fn get_attachment(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, AttachmentId)>,
) -> ApiResult<Response> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let (meta, data) = blocking(move || books.read(|c| attachments::get(c, id))).await?;
    // Served as a download unless it's something a browser shows safely inline.
    let inline = meta.mime.starts_with("image/") && meta.mime != "image/svg+xml"
        || meta.mime == "application/pdf";
    let disposition = format!(
        "{}; filename=\"{}\"",
        if inline { "inline" } else { "attachment" },
        meta.filename.replace(['"', '\\'], "_")
    );
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            if inline {
                meta.mime.as_str()
            } else {
                "application/octet-stream"
            },
        )
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(
            "content-security-policy",
            "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'",
        )
        .body(Body::from(data))
        .map_err(|e| ApiError::internal(e.to_string()))
}

#[derive(Deserialize)]
pub struct AttachBody {
    record_kind: Option<String>,
    record_id: Option<String>,
}

pub async fn attach(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, AttachmentId)>,
    Json(b): Json<AttachBody>,
) -> ApiResult<Json<AttachmentMeta>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || {
            books.write(&actor, |w| {
                attachments::attach(w, id, b.record_kind.as_deref().zip(b.record_id.as_deref()))
            })
        })
        .await?,
    ))
}

pub async fn delete_attachment(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, AttachmentId)>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let actor = p.actor();
    blocking(move || books.write(&actor, |w| attachments::delete(w, id))).await?;
    Ok(StatusCode::NO_CONTENT)
}
