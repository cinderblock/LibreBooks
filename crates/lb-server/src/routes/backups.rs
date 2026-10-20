//! One company's backups: list, back up now, verify, restore (as a new company),
//! and download the portable `.lbk` company file. All of it needs admin on the
//! company; restoring also creates a company, so it needs a server admin too.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use lb_backup::{BackupRecord, VerifyReport};
use lb_core::CompanyId;
use serde::Deserialize;
use serde_json::{Value, json};

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::ops::backups::{self, BackupList, Restored, UnlockRequest};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/backups", get(list))
        .route("/backups/status", get(status))
        .route("/backups/run", post(run_now))
        .route("/backups/export", get(export))
        .route("/backups/{id}/verify", post(verify))
        .route("/backups/{id}/restore", post(restore))
}

async fn company_name(books: &lb_store::Books) -> ApiResult<String> {
    let b = books.clone();
    Ok(blocking(move || b.read(lb_store::company::info))
        .await?
        .name)
}

async fn list(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<BackupList>> {
    s.books_for(&p, company, Role::Admin)?;
    Ok(Json(backups::list(&s, company).await))
}

async fn status(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Value>> {
    s.books_for(&p, company, Role::Admin)?;
    let settings = s.ops.settings();
    let config = backups::company_config(&s, company);
    Ok(Json(json!({
        "enabled": settings.backups_enabled() && settings.backups.schedule.enabled,
        "destinations": config.destinations.iter().map(lb_backup::Destination::describe).collect::<Vec<_>>(),
        "scheduler": backups::status(&s, company),
    })))
}

async fn run_now(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<BackupRecord>> {
    let books = s.books_for(&p, company, Role::Admin)?;
    let name = company_name(&books).await?;
    Ok(Json(backups::run_now(&s, company, &books, &name).await?))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Which {
    /// Which destination (its position in the settings); any by default.
    destination: Option<usize>,
}

async fn verify(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, String)>,
    Query(which): Query<Which>,
    body: Option<Json<UnlockRequest>>,
) -> ApiResult<Json<VerifyReport>> {
    s.books_for(&p, company, Role::Admin)?;
    let entry = backups::find(&s, company, &id, which.destination).await?;
    let unlock = body.map(|b| b.0).unwrap_or_default().unlock(&s)?;
    Ok(Json(backups::verify(&s, &entry, &unlock).await?))
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RestoreBody {
    /// The restored company's name; default "<name> (restored from <date>)".
    name: Option<String>,
    #[serde(flatten)]
    unlock: UnlockRequest,
}

async fn restore(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, String)>,
    Query(which): Query<Which>,
    body: Option<Json<RestoreBody>>,
) -> ApiResult<(StatusCode, Json<Restored>)> {
    p.require(Role::Admin)?;
    s.books_for(&p, company, Role::Admin)?;
    let body = body.map(|b| b.0).unwrap_or_default();
    let entry = backups::find(&s, company, &id, which.destination).await?;
    let unlock = body.unlock.unlock(&s)?;
    let restored = backups::restore(&s, &entry, &unlock, body.name, &p.actor()).await?;
    Ok((StatusCode::CREATED, Json(restored)))
}

/// `GET /backups/export`: the company as a `.lbk` file (a zip of the database
/// and a manifest) to keep or move to another server.
async fn export(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Response> {
    let books = s.books_for(&p, company, Role::Admin)?;
    let name = company_name(&books).await?;
    let work = tempfile::Builder::new()
        .prefix(".lb-export-")
        .tempdir_in(s.ops.data_dir())
        .map_err(|e| ApiError::internal(format!("could not make a working folder: {e}")))?;
    let path = work.path().join("company.lbk");
    backups::export_lbk(&books, &name, path.clone()).await?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::internal(format!("could not read the export: {e}")))?;
    drop(work);
    let file = format!("{}.lbk", lb_backup::slug(&name));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{file}\""),
        )
        .body(Body::from(bytes))
        .map_err(|e| ApiError::internal(e.to_string()))
}
