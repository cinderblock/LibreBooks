//! Health, the session, and the company list.

use axum::Json;
use axum::extract::State;
use lb_core::CompanyInfo;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Principal, Role};
use crate::error::ApiResult;
use crate::state::AppState;

pub async fn health(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "ok": true, "version": s.version, "mode": s.mode }))
}

pub async fn session(State(s): State<AppState>, p: Principal) -> ApiResult<Json<Value>> {
    let companies = s.companies.list()?;
    let role = p.role;
    Ok(Json(json!({
        "user": { "id": p.user_id, "display_name": p.display_name, "role": role },
        "companies": companies.into_iter().map(|c| json!({ "id": c.id, "name": c.name, "role": role })).collect::<Vec<_>>(),
    })))
}

pub async fn companies(State(s): State<AppState>, p: Principal) -> ApiResult<Json<Value>> {
    let role = p.role;
    let list = s.companies.list()?;
    Ok(Json(json!(
        list.into_iter()
            .map(|c| json!({ "id": c.id, "name": c.name, "role": role }))
            .collect::<Vec<_>>()
    )))
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
