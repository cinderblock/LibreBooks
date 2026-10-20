//! Health, the session, the company list, and bringing a company in from a
//! `.lbk` file.

use axum::Json;
use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use lb_core::CompanyInfo;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::ops::backups;
use crate::state::AppState;

/// The most a `.lbk` upload may be: well past any company file SQLite would
/// still be a sensible home for.
pub const IMPORT_BODY_LIMIT: usize = 8 << 30;

pub async fn health(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "ok": true, "version": s.version, "mode": s.mode }))
}

pub async fn session(State(s): State<AppState>, p: Principal) -> ApiResult<Json<Value>> {
    let companies = companies_json(&s, &p)?;
    let kind = p.session.as_ref().map(|sp| sp.kind);
    Ok(Json(json!({
        "user": { "id": p.user_id, "display_name": p.display_name, "role": p.role, "kind": kind },
        "companies": companies,
        "sign_in": s.sessions.is_some(),
    })))
}

pub async fn companies(State(s): State<AppState>, p: Principal) -> ApiResult<Json<Value>> {
    Ok(Json(companies_json(&s, &p)?))
}

fn companies_json(s: &AppState, p: &Principal) -> ApiResult<Value> {
    Ok(json!(
        s.companies_for(p)?
            .into_iter()
            .map(|(c, role)| json!({ "id": c.id, "name": c.name, "role": role }))
            .collect::<Vec<_>>()
    ))
}

#[derive(Deserialize)]
pub struct CreateCompany {
    info: CompanyInfo,
}

pub async fn create_company(
    State(s): State<AppState>,
    p: Principal,
    Json(body): Json<CreateCompany>,
) -> ApiResult<Json<Value>> {
    p.require(Role::Admin)?;
    let id = tokio::task::spawn_blocking(move || s.companies.create(&body.info)).await??;
    Ok(Json(json!({ "id": id })))
}

/// `POST /companies/import` (multipart `file`: a `.lbk` from `GET
/// /c/:company/backups/export` or the desktop app; `name?`): the file becomes a
/// **new** company with a fresh id. Server admin. The upload streams to a
/// working file under the data folder, so a large company doesn't sit in
/// memory.
pub async fn import_company(
    State(s): State<AppState>,
    p: Principal,
    mut form: Multipart,
) -> ApiResult<(StatusCode, Json<Value>)> {
    p.require(Role::Admin)?;
    let work = s.ops.data_dir().join(".restore");
    tokio::fs::create_dir_all(&work)
        .await
        .map_err(|e| ApiError::internal(format!("could not create {}: {e}", work.display())))?;
    let upload = work.join(format!("{}.lbk", uuid::Uuid::now_v7()));
    let mut name: Option<String> = None;
    let mut received = false;
    let outcome: ApiResult<()> = async {
        while let Some(mut field) = form
            .next_field()
            .await
            .map_err(|e| ApiError::invalid(e.to_string()))?
        {
            match field.name() {
                Some("file") => {
                    let mut out = tokio::fs::File::create(&upload).await.map_err(|e| {
                        ApiError::internal(format!("could not write {}: {e}", upload.display()))
                    })?;
                    while let Some(chunk) = field
                        .chunk()
                        .await
                        .map_err(|e| ApiError::invalid(e.to_string()))?
                    {
                        received |= !chunk.is_empty();
                        out.write_all(&chunk).await.map_err(|e| {
                            ApiError::internal(format!("could not write the upload: {e}"))
                        })?;
                    }
                    out.flush().await.map_err(|e| {
                        ApiError::internal(format!("could not write the upload: {e}"))
                    })?;
                }
                Some("name") => {
                    name = Some(
                        field
                            .text()
                            .await
                            .map_err(|e| ApiError::invalid(e.to_string()))?,
                    );
                }
                _ => {}
            }
        }
        if !received {
            return Err(ApiError::invalid("no file was sent"));
        }
        Ok(())
    }
    .await;
    let imported = match outcome {
        Ok(()) => backups::import_lbk(&s, upload.clone(), name, &p.actor()).await,
        Err(e) => Err(e),
    };
    let _ = tokio::fs::remove_file(&upload).await;
    let (id, manifest) = imported?;
    let books = s.companies.get(id)?;
    let name = tokio::task::spawn_blocking(move || {
        books.read(|c| lb_store::company::info(c).map(|i| i.name))
    })
    .await??;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "name": name, "from": manifest })),
    ))
}
