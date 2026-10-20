//! QuickBooks Desktop import: a company's import settings, starting a sync or a
//! parity check (one job at a time, in the background), their status and
//! reports, the Web Connector's `.qwc` file and password, all admin on the
//! company; and the Web Connector's SOAP endpoint, `/qbwc`, which has its own
//! sign-in (user name + password from the `.qwc`) instead of a session cookie.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lb_core::CompanyId;
use lb_qbd::config::{ImportSettings, QbdTransport};
use lb_qbd::qbwc::{Call, soap};
use serde::Deserialize;
use serde_json::{Value, json};

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::import::{self, ImportStatus, ParityBody, SyncBody};
use crate::state::AppState;

/// The company-scoped routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/import/qbd", get(get_status).put(put_settings))
        .route("/import/qbd/status", get(get_status))
        .route("/import/qbd/sync", post(start_sync))
        .route("/import/qbd/parity", get(get_parity).post(start_parity))
        .route("/import/qbd/qbwc.qwc", get(qwc_file))
        .route("/import/qbd/qbwc/password", post(set_password))
}

/// The Web Connector's endpoint, mounted at the root (outside `/api/v1`).
pub fn qbwc() -> Router<AppState> {
    Router::new().route("/qbwc", get(qbwc_get).post(qbwc_post))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Wait {
    /// Long-poll: wait up to this many seconds for a running job to finish.
    wait: Option<u64>,
}

impl Wait {
    fn duration(&self) -> Duration {
        Duration::from_secs(self.wait.unwrap_or(0).min(120))
    }
}

async fn status_of(
    s: &AppState,
    company: CompanyId,
    books: &lb_store::Books,
) -> ApiResult<ImportStatus> {
    import::status(s, company, books).await
}

async fn get_status(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(wait): Query<Wait>,
) -> ApiResult<Json<ImportStatus>> {
    let books = s.books_for(&p, company, Role::Admin)?;
    import::wait_for_idle(&s, company, wait.duration()).await;
    Ok(Json(status_of(&s, company, &books).await?))
}

/// `PUT …/import/qbd`: the settings, plus the write-only `bridge_key`.
#[derive(Deserialize)]
struct SettingsBody {
    #[serde(flatten)]
    settings: ImportSettings,
    /// The bridge's API key, saved in the secret store; never returned.
    #[serde(default)]
    bridge_key: Option<String>,
}

impl std::fmt::Debug for SettingsBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsBody")
            .field("settings", &self.settings)
            .field(
                "bridge_key",
                &self.bridge_key.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

async fn put_settings(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(body): Json<SettingsBody>,
) -> ApiResult<Json<ImportStatus>> {
    let books = s.books_for(&p, company, Role::Admin)?;
    let b = books.clone();
    let stored = blocking(move || Ok(import::settings_of(&b, company))).await??;
    let mut settings = body.settings;
    // The `.qwc` ids are minted once; a client can't rotate them by omission.
    settings.qbwc.owner_id = stored.qbwc.owner_id;
    settings.qbwc.file_id = stored.qbwc.file_id;
    settings.qbwc.user_name = settings.qbwc.user_name.trim().to_owned();
    if settings.qbwc.user_name.is_empty() {
        settings.qbwc.user_name = stored.qbwc.user_name.clone();
    }
    settings.bridge.url = settings.bridge.url.trim().to_owned();

    let store = s.ops.secrets.clone();
    let key_name = import::bridge_key_name(company);
    if let Some(key) = body.bridge_key.filter(|k| !k.trim().is_empty()) {
        let (store, name) = (store.clone(), key_name.clone());
        tokio::task::spawn_blocking(move || store.set(&name, key.trim())).await??;
        settings.bridge.key_ref = Some(format!("secret:{key_name}"));
    } else if settings.bridge.key_ref.is_none() {
        let (store, name) = (store.clone(), key_name.clone());
        if tokio::task::spawn_blocking(move || store.contains(&name)).await?? {
            settings.bridge.key_ref = Some(format!("secret:{key_name}"));
        }
    }
    if let Some(r) = &settings.bridge.key_ref {
        let (store, r) = (store.clone(), r.clone());
        tokio::task::spawn_blocking(move || store.check_reference(&r))
            .await?
            .map_err(|e| ApiError::invalid(format!("the bridge key reference is unusable: {e}")))?;
    }
    let problems = settings.problems();
    if !problems.is_empty() {
        return Err(
            ApiError::invalid(problems.join("; ")).with_detail(json!({ "problems": problems }))
        );
    }
    if settings.transport == QbdTransport::Qbwc {
        let (s2, name) = (s.clone(), settings.qbwc.user_name.clone());
        if tokio::task::spawn_blocking(move || import::user_name_taken(&s2, company, &name))
            .await??
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "duplicate",
                format!(
                    "another company already uses the Web Connector user name {:?}",
                    settings.qbwc.user_name
                ),
            ));
        }
    }
    let (b, to_save) = (books.clone(), settings.clone());
    blocking(move || Ok(import::save_settings(&b, &to_save))).await??;
    Ok(Json(status_of(&s, company, &books).await?))
}

async fn start_sync(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    body: Option<Json<SyncBody>>,
) -> ApiResult<(StatusCode, Json<ImportStatus>)> {
    let books = s.books_for(&p, company, Role::Admin)?;
    let body = body.map(|b| b.0).unwrap_or_default();
    import::start_sync(&s, company, books.clone(), body).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(status_of(&s, company, &books).await?),
    ))
}

async fn start_parity(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    body: Option<Json<ParityBody>>,
) -> ApiResult<(StatusCode, Json<ImportStatus>)> {
    let books = s.books_for(&p, company, Role::Admin)?;
    let body = body.map(|b| b.0).unwrap_or_default();
    import::start_parity(&s, company, books.clone(), body).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(status_of(&s, company, &books).await?),
    ))
}

/// `GET …/import/qbd/parity`: the last parity report and whether one is running.
async fn get_parity(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(wait): Query<Wait>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Admin)?;
    import::wait_for_idle(&s, company, wait.duration()).await;
    let status = status_of(&s, company, &books).await?;
    Ok(Json(json!({
        "running": status.running.as_ref().is_some_and(|r| r.kind == import::JobKind::Parity),
        "job": status.running,
        "report": status.last_parity,
        "error": status.last_error,
    })))
}

/// `GET …/import/qbd/qbwc.qwc`: the file to add to the QuickBooks Web Connector.
async fn qwc_file(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let books = s.books_for(&p, company, Role::Admin)?;
    let b = books.clone();
    let (settings, info) = blocking(move || {
        let settings = import::settings_of(&b, company);
        let info = b.read(lb_store::company::info)?;
        Ok((settings, info))
    })
    .await?;
    let settings = settings?;
    let origin = request_origin(&headers);
    let file = import::qwc_file(&s, company, &info.name, &settings, origin.as_deref())
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let name = format!("librebooks-{}.qwc", lb_backup::slug(&info.name));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(Body::from(file.to_xml()))
        .map_err(|e| ApiError::internal(e.to_string()))
}

/// The origin the request came to, from the proxy's headers when there is one.
fn request_origin(headers: &HeaderMap) -> Option<String> {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))?
        .to_str()
        .ok()?;
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    Some(format!("{scheme}://{host}"))
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PasswordBody {
    value: Option<String>,
}

impl std::fmt::Debug for PasswordBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordBody([redacted])")
    }
}

/// `POST …/import/qbd/qbwc/password`: set the Web Connector password (`{ value }`
/// → 204), or have one generated (empty body → `{ password }`, shown this once).
async fn set_password(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    body: Option<Json<PasswordBody>>,
) -> ApiResult<Response> {
    s.books_for(&p, company, Role::Admin)?;
    let given = body.and_then(|b| b.0.value).map(|v| v.trim().to_owned());
    if given.as_deref().is_some_and(|v| v.len() < 8) {
        return Err(ApiError::invalid(
            "the Web Connector password needs at least 8 characters",
        ));
    }
    let generated = given.is_none();
    let password = given.unwrap_or_else(import::new_password);
    let store = s.ops.secrets.clone();
    let name = import::qbwc_password_name(company);
    let to_store = password.clone();
    tokio::task::spawn_blocking(move || store.set(&name, &to_store)).await??;
    if generated {
        Ok(Json(json!({ "password": password })).into_response())
    } else {
        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

// ---------------------------------------------------------------------------
// The Web Connector's SOAP endpoint
// ---------------------------------------------------------------------------

/// A browser check of the URL in the `.qwc`.
async fn qbwc_get(State(s): State<AppState>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!(
            "LibreBooks {} QuickBooks Web Connector endpoint.\nAdd this company's .qwc file to the Web Connector; it POSTs SOAP here.\n",
            s.version
        ),
    )
        .into_response()
}

fn soap_response(xml: String) -> Response {
    ([(header::CONTENT_TYPE, soap::CONTENT_TYPE)], xml).into_response()
}

/// Every Web Connector method. Each answer is a SOAP envelope; a body that
/// isn't one gets a SOAP fault.
async fn qbwc_post(State(s): State<AppState>, body: String) -> Response {
    let call = match soap::parse_call(&body) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Web Connector sent an unreadable request");
            return (
                StatusCode::BAD_REQUEST,
                soap_response(soap::fault(&e.to_string())),
            )
                .into_response();
        }
    };
    tracing::debug!(?call, "Web Connector");
    let method = call.method();
    let xml = match call {
        Call::ServerVersion => soap::string_response(method, &format!("LibreBooks {}", s.version)),
        // "" accepts the Web Connector's version; "W:…" would warn, "E:…" refuse.
        Call::ClientVersion { .. } => soap::string_response(method, ""),
        Call::Authenticate {
            user_name,
            password,
        } => {
            let (ticket, outcome) = import::authenticate(&s, &user_name, &password).await;
            soap::authenticate_response(&ticket, &outcome)
        }
        Call::SendRequestXml {
            ticket,
            company_file,
            qbxml_major,
            qbxml_minor,
            ..
        } => match import::session_for(&s, &ticket) {
            Some(session) => {
                tracing::debug!(
                    company_file,
                    qbxml = format!("{qbxml_major}.{qbxml_minor}"),
                    "Web Connector asks for work"
                );
                soap::string_response(method, &session.send_request_xml().await)
            }
            None => soap::string_response(method, ""),
        },
        Call::ReceiveResponseXml {
            ticket,
            response,
            hresult,
            message,
        } => match import::session_for(&s, &ticket) {
            Some(session) => soap::int_response(
                method,
                session
                    .receive_response_xml(response, &hresult, &message)
                    .await,
            ),
            None => soap::int_response(method, -1),
        },
        Call::GetLastError { ticket } => match import::session_for(&s, &ticket) {
            Some(session) => soap::string_response(method, &session.get_last_error()),
            None => soap::string_response(method, "LibreBooks doesn't know this session ticket"),
        },
        Call::ConnectionError {
            ticket,
            hresult,
            message,
        } => match import::session_for(&s, &ticket) {
            Some(session) => {
                soap::string_response(method, &session.connection_error(&hresult, &message))
            }
            None => soap::string_response(method, "done"),
        },
        Call::CloseConnection { ticket } => {
            let summary = match import::session_for(&s, &ticket) {
                Some(session) => session.close_connection(),
                None => "LibreBooks: no session".to_owned(),
            };
            import::close_session(&s, &ticket);
            soap::string_response(method, &summary)
        }
    };
    soap_response(xml)
}
