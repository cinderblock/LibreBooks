//! Sign-in, enrolment, and managing people, passkeys, devices and sessions:
//! `lb-auth` mapped onto `plans/api.md`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lb_auth::{
    AuthStore, CompanyId, DeviceEnrol, DeviceKeyId, DeviceLogin, EnrolFinish, EnrolStartRequest,
    EnrolmentId, LoginFinish, NewEnrolment, NewSession, PasskeyId, Role, SessionId, User, UserId,
    UserUpdate,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Client, Principal, RequestOrigin, session_cookie};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

fn store(s: &AppState) -> ApiResult<AuthStore> {
    s.sessions.as_ref().map(|x| x.store.clone()).ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "this server doesn't use sign-in",
        )
    })
}

/// Run blocking auth work off the async runtime.
async fn run<T, F>(f: F) -> ApiResult<T>
where
    F: FnOnce() -> lb_auth::Result<T> + Send + 'static,
    T: Send + 'static,
{
    Ok(tokio::task::spawn_blocking(f).await??)
}

/// A JSON response that also sets the session cookie.
fn signed_in(origin: &str, user: &User, session: &NewSession) -> Response {
    let max_age =
        (session.info.expires_at.as_second() - jiff::Timestamp::now().as_second()).max(0) as u64;
    let mut res = Json(json!({ "user": user })).into_response();
    if let Ok(v) = HeaderValue::from_str(&session_cookie(Some(&session.token), origin, max_age)) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

// --- Sign-in ---------------------------------------------------------------

pub async fn login_start(
    State(s): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    Client(client): Client,
) -> ApiResult<Json<lb_auth::LoginStart>> {
    let st = store(&s)?;
    Ok(Json(run(move || st.start_login(&origin, &client)).await?))
}

pub async fn login_finish(
    State(s): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    Client(client): Client,
    Json(body): Json<LoginFinish>,
) -> ApiResult<Response> {
    let st = store(&s)?;
    let o = origin.clone();
    let (user, session) = run(move || st.finish_login(&body, &o, &client)).await?;
    Ok(signed_in(&origin, &user, &session))
}

pub async fn enroll_inspect(
    State(s): State<AppState>,
    Client(client): Client,
    Query(q): Query<TokenQuery>,
) -> ApiResult<Json<lb_auth::EnrolmentInfo>> {
    let st = store(&s)?;
    Ok(Json(
        run(move || st.inspect_enrolment(&q.token, &client)).await?,
    ))
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: String,
}

pub async fn enroll_start(
    State(s): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    Client(client): Client,
    Json(body): Json<EnrolStartRequest>,
) -> ApiResult<Json<lb_auth::EnrolStart>> {
    let st = store(&s)?;
    Ok(Json(
        run(move || st.start_enrolment(&body, &origin, &client)).await?,
    ))
}

pub async fn enroll_finish(
    State(s): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    Client(client): Client,
    Json(body): Json<EnrolFinish>,
) -> ApiResult<Response> {
    let st = store(&s)?;
    let o = origin.clone();
    let (user, session) = run(move || st.finish_enrolment(&body, &o, &client)).await?;
    Ok(signed_in(&origin, &user, &session))
}

pub async fn device_challenge(
    State(s): State<AppState>,
    Client(client): Client,
) -> ApiResult<Json<lb_auth::DeviceChallenge>> {
    let st = store(&s)?;
    Ok(Json(run(move || st.device_challenge(&client)).await?))
}

/// Device login answers with the session token in the body as well as the
/// cookie: thin clients keep it themselves.
pub async fn device_login(
    State(s): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    Client(client): Client,
    Json(body): Json<DeviceLogin>,
) -> ApiResult<Response> {
    let st = store(&s)?;
    let o = origin.clone();
    let (user, session) = run(move || st.device_login(&body, &o, &client)).await?;
    let max_age =
        (session.info.expires_at.as_second() - jiff::Timestamp::now().as_second()).max(0) as u64;
    let mut res = Json(
        json!({ "user": user, "token": session.token, "expires_at": session.info.expires_at }),
    )
    .into_response();
    if let Ok(v) = HeaderValue::from_str(&session_cookie(Some(&session.token), &origin, max_age)) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    Ok(res)
}

pub async fn device_enroll(
    State(s): State<AppState>,
    Client(client): Client,
    Json(body): Json<DeviceEnrol>,
) -> ApiResult<Json<Value>> {
    let st = store(&s)?;
    let (user, key) = run(move || st.enrol_device(&body, &client)).await?;
    Ok(Json(json!({ "user": user, "key": key })))
}

pub async fn logout(
    State(s): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let token = crate::auth::token_from_headers(&headers);
    if let (Some(t), Ok(st)) = (token, store(&s))
        && t.starts_with("lbs_")
    {
        run(move || st.logout(&t)).await?;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    if let Ok(v) = HeaderValue::from_str(&session_cookie(None, &origin, 0)) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    Ok(res)
}

// --- My passkeys, devices and sessions ----------------------------------------

pub async fn my_passkeys(
    State(s): State<AppState>,
    p: Principal,
) -> ApiResult<Json<Vec<lb_auth::PasskeyInfo>>> {
    let me = p.person()?.user_id;
    let st = store(&s)?;
    Ok(Json(run(move || st.list_passkeys(me)).await?))
}

#[derive(Deserialize)]
pub struct Rename {
    name: String,
}

pub async fn rename_passkey(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<PasskeyId>,
    Json(b): Json<Rename>,
) -> ApiResult<StatusCode> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    run(move || st.rename_passkey(&by, id, &b.name)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_passkey(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<PasskeyId>,
) -> ApiResult<StatusCode> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    run(move || st.delete_passkey(&by, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn my_devices(
    State(s): State<AppState>,
    p: Principal,
) -> ApiResult<Json<Vec<lb_auth::DeviceKeyInfo>>> {
    let me = p.person()?.user_id;
    let st = store(&s)?;
    Ok(Json(run(move || st.list_device_keys(me)).await?))
}

pub async fn revoke_device(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<DeviceKeyId>,
) -> ApiResult<StatusCode> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    run(move || st.revoke_device_key(&by, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn my_sessions(
    State(s): State<AppState>,
    p: Principal,
) -> ApiResult<Json<Vec<lb_auth::SessionInfo>>> {
    let me = p.person()?.clone();
    let st = store(&s)?;
    Ok(Json(
        run(move || st.list_sessions(me.user_id, Some(me.session_id))).await?,
    ))
}

pub async fn revoke_session(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<SessionId>,
) -> ApiResult<StatusCode> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    run(move || st.revoke_session(&by, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Enrolment links --------------------------------------------------------

/// A minted enrolment and the link to share (or show as a QR code).
#[derive(Clone, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct MintedEnrolmentLink {
    #[serde(flatten)]
    #[ts(flatten)]
    pub minted: lb_auth::MintedEnrolment,
    pub url: String,
}

/// Mint a link: the response's `url` is what to share (or show as a QR code).
pub async fn create_enrollment(
    State(s): State<AppState>,
    p: Principal,
    RequestOrigin(origin): RequestOrigin,
    Json(b): Json<NewEnrolment>,
) -> ApiResult<Json<MintedEnrolmentLink>> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    let base = s
        .sessions
        .as_ref()
        .and_then(|x| x.public_origin.clone())
        .unwrap_or(origin);
    let minted = run(move || st.create_enrolment(&by, &b)).await?;
    let url = enrolment_url(&base, &minted.token);
    Ok(Json(MintedEnrolmentLink { minted, url }))
}

/// Where an enrolment link points: the web UI's `/enroll` page.
pub fn enrolment_url(base: &str, token: &str) -> String {
    format!("{}/enroll#token={token}", base.trim_end_matches('/'))
}

pub async fn list_enrollments(
    State(s): State<AppState>,
    p: Principal,
) -> ApiResult<Json<Vec<lb_auth::EnrolmentInfo>>> {
    p.require(Role::Admin)?;
    let st = store(&s)?;
    Ok(Json(run(move || st.list_enrolments()).await?))
}

pub async fn revoke_enrollment(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<EnrolmentId>,
) -> ApiResult<StatusCode> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    run(move || st.revoke_enrolment(&by, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- People (admins) --------------------------------------------------------

pub async fn list_users(State(s): State<AppState>, p: Principal) -> ApiResult<Json<Vec<User>>> {
    p.require(Role::Admin)?;
    let st = store(&s)?;
    Ok(Json(run(move || st.list_users()).await?))
}

pub async fn get_user(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<UserId>,
) -> ApiResult<Json<Value>> {
    let me = p.person()?.user_id;
    if me != id {
        p.require(Role::Admin)?;
    }
    let st = store(&s)?;
    let (user, access, passkeys, devices) = run(move || {
        Ok((
            st.user(id)?,
            st.company_access(id)?,
            st.list_passkeys(id)?,
            st.list_device_keys(id)?,
        ))
    })
    .await?;
    Ok(Json(
        json!({ "user": user, "companies": access, "passkeys": passkeys, "devices": devices }),
    ))
}

pub async fn update_user(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<UserId>,
    Json(b): Json<UserUpdate>,
) -> ApiResult<Json<User>> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    Ok(Json(run(move || st.update_user(&by, id, &b)).await?))
}

pub async fn delete_user(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<UserId>,
) -> ApiResult<StatusCode> {
    let by = p.person()?.clone();
    let st = store(&s)?;
    run(move || st.delete_user(&by, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct Access {
    role: Role,
}

pub async fn grant_access(
    State(s): State<AppState>,
    p: Principal,
    Path((id, company)): Path<(UserId, CompanyId)>,
    Json(b): Json<Access>,
) -> ApiResult<StatusCode> {
    p.require(Role::Admin)?;
    s.companies.get(company)?;
    let st = store(&s)?;
    run(move || st.grant_company_access(id, company, b.role)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_access(
    State(s): State<AppState>,
    p: Principal,
    Path((id, company)): Path<(UserId, CompanyId)>,
) -> ApiResult<StatusCode> {
    p.require(Role::Admin)?;
    let st = store(&s)?;
    run(move || st.revoke_company_access(id, company)).await?;
    Ok(StatusCode::NO_CONTENT)
}
