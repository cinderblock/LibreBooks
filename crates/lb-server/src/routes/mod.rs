//! The `/api/v1` routes. See `plans/api.md` for the contract.

use axum::Router;
use axum::routing::{get, post, put};

use crate::state::AppState;

mod auth;
mod backups;
mod banking;
mod company;
mod events;
mod feeds;
mod import;
mod lists;
mod merge;
mod ops;
mod reports;
mod saved;
mod system;
mod txns;

/// The QuickBooks Web Connector's SOAP endpoint (`/qbwc`), mounted at the root
/// by `lib.rs`: it has its own sign-in, not the API's cookie.
pub fn qbwc() -> Router<AppState> {
    import::qbwc()
}

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
        .route("/txns/{id}/fulfilment", get(txns::fulfilment))
        .route("/accounts/{id}/register", get(txns::register))
        .route(
            "/accounts/{id}/register/{txn}/cleared",
            put(txns::set_cleared),
        )
        .route("/balances", get(txns::balances))
        .route("/balances/names", get(txns::name_balances))
        .route("/open", get(txns::open_items))
        .route("/undeposited", get(txns::undeposited))
        .route("/audit", get(company::audit))
        .route("/merge", post(merge::merge))
        .route("/memorized/due", get(saved::due))
        .route("/memorized/{id}/use", get(saved::use_memorized))
        .route("/saved/{kind}", get(saved::list).post(saved::create))
        .route(
            "/saved/{kind}/{id}",
            put(saved::update).delete(saved::delete),
        )
        .route("/reports", get(reports::catalog))
        .route("/reports/run", post(reports::run_report))
        .route("/reports/export", post(reports::export))
        .route(
            "/accounts/{id}/reconcile",
            get(banking::reconcile_overview).post(banking::reconcile_start),
        )
        .route(
            "/accounts/{id}/reconcile/undo",
            post(banking::reconcile_undo),
        )
        .route(
            "/accounts/{id}/opening-balance",
            post(banking::opening_balance),
        )
        .route(
            "/reconciliations/{id}",
            get(banking::reconciliation).delete(banking::reconcile_discard),
        )
        .route("/reconciliations/{id}/mark", post(banking::reconcile_mark))
        .route(
            "/reconciliations/{id}/finish",
            post(banking::reconcile_finish),
        )
        .route(
            "/attachments",
            get(banking::list_attachments).post(banking::upload),
        )
        .route(
            "/attachments/{id}",
            get(banking::get_attachment)
                .put(banking::attach)
                .delete(banking::delete_attachment),
        )
        // Bank feeds, backups and the QuickBooks import (routes/feeds.rs,
        // routes/backups.rs, routes/import.rs).
        .merge(feeds::routes())
        .merge(backups::routes())
        .merge(import::routes());
    Router::new()
        // Server administration (routes/ops.rs).
        .merge(ops::routes())
        .route("/health", get(system::health))
        .route("/session", get(system::session))
        .route(
            "/companies",
            get(system::companies).post(system::create_company),
        )
        .route(
            "/companies/import",
            post(system::import_company).layer(axum::extract::DefaultBodyLimit::max(
                system::IMPORT_BODY_LIMIT,
            )),
        )
        .route("/events", get(events::stream))
        .route("/auth/login/start", post(auth::login_start))
        .route("/auth/login/finish", post(auth::login_finish))
        .route("/auth/enroll", get(auth::enroll_inspect))
        .route("/auth/enroll/start", post(auth::enroll_start))
        .route("/auth/enroll/finish", post(auth::enroll_finish))
        .route("/auth/device/challenge", post(auth::device_challenge))
        .route("/auth/device/login", post(auth::device_login))
        .route("/auth/device/enroll", post(auth::device_enroll))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/passkeys", get(auth::my_passkeys))
        .route(
            "/auth/passkeys/{id}",
            axum::routing::patch(auth::rename_passkey).delete(auth::delete_passkey),
        )
        .route("/auth/devices", get(auth::my_devices))
        .route(
            "/auth/devices/{id}",
            axum::routing::delete(auth::revoke_device),
        )
        .route("/auth/sessions", get(auth::my_sessions))
        .route(
            "/auth/sessions/{id}",
            axum::routing::delete(auth::revoke_session),
        )
        .route(
            "/auth/enrollments",
            get(auth::list_enrollments).post(auth::create_enrollment),
        )
        .route(
            "/auth/enrollments/{id}",
            axum::routing::delete(auth::revoke_enrollment),
        )
        .route("/users", get(auth::list_users))
        .route(
            "/users/{id}",
            get(auth::get_user)
                .patch(auth::update_user)
                .delete(auth::delete_user),
        )
        .route(
            "/users/{id}/companies/{company}",
            put(auth::grant_access).delete(auth::revoke_access),
        )
        .nest("/c/{company}", company)
}
