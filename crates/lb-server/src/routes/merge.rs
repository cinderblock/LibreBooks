//! Merging two names, accounts or items into one (QBD's rename-to-merge).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lb_core::CompanyId;
use lb_store::merge;
use serde::Deserialize;

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct MergeBody {
    /// `account`, `entity` or `item`.
    kind: String,
    from: String,
    into: String,
}

pub async fn merge(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(b): Json<MergeBody>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Admin)?;
    let actor = p.actor();
    let bad = || ApiError::invalid("from and into must be ids");
    match b.kind.as_str() {
        "account" => {
            let (f, i) = (
                b.from.parse().map_err(|_| bad())?,
                b.into.parse().map_err(|_| bad())?,
            );
            blocking(move || books.write(&actor, |w| merge::merge_accounts(w, f, i))).await?;
        }
        "entity" => {
            let (f, i) = (
                b.from.parse().map_err(|_| bad())?,
                b.into.parse().map_err(|_| bad())?,
            );
            blocking(move || books.write(&actor, |w| merge::merge_entities(w, f, i))).await?;
        }
        "item" => {
            let (f, i) = (
                b.from.parse().map_err(|_| bad())?,
                b.into.parse().map_err(|_| bad())?,
            );
            blocking(move || books.write(&actor, |w| merge::merge_items(w, f, i))).await?;
        }
        other => {
            return Err(ApiError::invalid(format!(
                "can't merge {other}; account, entity or item"
            )));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
