//! The `/api/v1` routes. See `plans/api.md` for the contract.

use axum::Router;
use axum::routing::{get, post, put};

use crate::state::AppState;

mod company;
mod events;
mod lists;
mod system;
mod txns;

/// Run blocking store work off the async runtime.
pub(crate) async fn blocking<T, F>(f: F) -> crate::error::ApiResult<T>
where
    F: FnOnce() -> Result<T, lb_store::StoreError> + Send + 'static,
    T: Send + 'static,
{
    Ok(tokio::task::spawn_blocking(f).await??)
}

pub fn api() -> Router<AppState> {
    let company = Router::new()
        .route("/info", get(company::get_info).put(company::put_info))
        .route(
            "/closing-date",
            get(company::get_closing_date).put(company::put_closing_date),
        )
        .merge(lists::routes())
        .route("/txns", get(txns::query).post(txns::create))
        .route("/txns/preview", post(txns::preview))
        .route("/txns/next-number", get(txns::next_number))
        .route(
            "/txns/{id}",
            get(txns::get).put(txns::update).delete(txns::delete),
        )
        .route("/txns/{id}/void", post(txns::void))
        .route("/txns/{id}/history", get(txns::history))
        .route("/accounts/{id}/register", get(txns::register))
        .route(
            "/accounts/{id}/register/{txn}/cleared",
            put(txns::set_cleared),
        )
        .route("/balances", get(txns::balances))
        .route("/open", get(txns::open_items))
        .route("/undeposited", get(txns::undeposited))
        .route("/audit", get(company::audit));
    Router::new()
        .route("/health", get(system::health))
        .route("/session", get(system::session))
        .route(
            "/companies",
            get(system::companies).post(system::create_company),
        )
        .route("/events", get(events::stream))
        .nest("/c/{company}", company)
}
