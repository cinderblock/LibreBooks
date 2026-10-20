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
