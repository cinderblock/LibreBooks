//! Bank Feeds: importing files, the review queue, matching and adding, rules,
//! online connections (SimpleFIN, OFX Direct Connect) and Amazon orders. The
//! route table is `plans/feeds.md`'s, plus `credentials` for saving a
//! connection's password.
//!
//! Credentials never travel back out. A connection's settings hold references,
//! and only to its own company's secrets (`secret:feeds.<company>.…`): a
//! bookkeeper may manage connections, and a reference to anything else (a server
//! secret, `env:`, `file:`) could send it to a server they control.

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lb_core::{AccountId, CompanyId, Money, Txn, TxnId};
use lb_feeds::add::{AddOutcome, AddRequest, BatchOutcome};
use lb_feeds::amazon::{AmazonMatch, AmazonOrder};
use lb_feeds::connections::{FeedConnection, ProviderSettings};
use lb_feeds::csvfile::{CsvDetection, CsvMapping};
use lb_feeds::ingest::{IngestOptions, IngestReport};
use lb_feeds::matching::{MatchCandidate, MatchOptions};
use lb_feeds::queue::{FeedAccountSummary, FeedFilter, FeedStatus, FeedTxn, ImportOptions};
use lb_feeds::rules::FeedRule;
use lb_feeds::{
    FeedConnectionId, FeedError, FeedRuleId, FeedTxnId, FileKind, Statement, StatementAccount,
    amazon, connections, csvfile, matching, ofx, providers, queue, rules,
};
use lb_store::{Books, StoreContext, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::secrets::CompanySecrets;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/feeds", get(summary))
        .route("/feeds/txns", get(list_txns))
        .route("/feeds/detect", post(detect))
        .route("/feeds/import", post(import))
        .route("/feeds/txns/add-all", post(add_all))
        .route("/feeds/txns/ignore", post(ignore))
        .route("/feeds/txns/{id}", get(get_txn))
        .route("/feeds/txns/{id}/candidates", get(candidates))
        .route("/feeds/txns/{id}/match", post(match_txn))
        .route("/feeds/txns/{id}/preview", post(preview))
        .route("/feeds/txns/{id}/add", post(add))
        .route("/feeds/txns/{id}/undo", post(undo))
        .route(
            "/feeds/connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/feeds/connections/{id}",
            get(get_connection)
                .put(update_connection)
                .delete(delete_connection),
        )
        .route("/feeds/connections/{id}/credentials", put(set_credentials))
        .route("/feeds/connections/{id}/sync", post(sync))
        .route(
            "/feeds/connections/{id}/remote-accounts",
            post(remote_accounts),
        )
        .route("/feeds/simplefin/claim", post(simplefin_claim))
        .route("/feeds/rules", get(list_rules).post(create_rule))
        .route("/feeds/rules/reorder", post(reorder_rules))
        .route("/feeds/rules/apply", post(apply_rules))
        .route(
            "/feeds/rules/{id}",
            get(get_rule).put(update_rule).delete(delete_rule),
        )
        .route("/feeds/amazon/orders", post(amazon_orders))
        .route("/feeds/amazon/apply", post(amazon_apply))
}

impl From<FeedError> for ApiError {
    fn from(e: FeedError) -> Self {
        let message = e.to_string();
        match e {
            FeedError::Parse { .. } | FeedError::Invalid(_) => ApiError::invalid(message),
            FeedError::NotFound { .. } => {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", message)
            }
            // The bank refused the saved credentials: the connection needs
            // reconnecting. Not 401, which would mean the person's own session.
            FeedError::Unauthorized { .. } => {
                ApiError::new(StatusCode::CONFLICT, "reconnect", message)
            }
            FeedError::Provider { .. } => {
                ApiError::new(StatusCode::BAD_GATEWAY, "provider", message)
            }
            FeedError::Network { .. } => ApiError::new(StatusCode::BAD_GATEWAY, "network", message),
            FeedError::Unsupported(_) => {
                ApiError::new(StatusCode::NOT_IMPLEMENTED, "unsupported", message)
            }
            FeedError::Store(s) => s.into(),
        }
    }
}

/// Run blocking feed work off the async runtime, keeping its error.
async fn fblocking<T, F>(f: F) -> ApiResult<T>
where
    F: FnOnce() -> lb_feeds::Result<T> + Send + 'static,
    T: Send + 'static,
{
    Ok(tokio::task::spawn_blocking(f).await??)
}

/// Read in a feed function.
async fn read<T: Send + 'static>(
    books: Books,
    f: impl FnOnce(&rusqlite::Connection) -> lb_feeds::Result<T> + Send + 'static,
) -> ApiResult<T> {
    fblocking(move || lb_feeds::read(&books, f)).await
}

/// Write with feed functions as `p`.
async fn write<T: Send + 'static>(
    books: Books,
    p: &Principal,
    f: impl FnOnce(&mut lb_store::Writer<'_>) -> lb_feeds::Result<T> + Send + 'static,
) -> ApiResult<T> {
    let actor = p.actor();
    fblocking(move || lb_feeds::write(&books, &actor, f)).await
}

// ---------------------------------------------------------------------------
// The queue

async fn summary(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Vec<FeedAccountSummary>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(read(books, queue::summary).await?))
}

/// `FeedFilter` as a query string (`statuses` comma-separated).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TxnQuery {
    account_id: Option<AccountId>,
    connection_id: Option<FeedConnectionId>,
    statuses: Option<String>,
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
    text: Option<String>,
    amount: Option<i64>,
    limit: Option<u32>,
    offset: Option<u32>,
    descending: bool,
}

impl TxnQuery {
    fn filter(self) -> ApiResult<FeedFilter> {
        let mut statuses = Vec::new();
        for s in self
            .statuses
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            statuses.push(FeedStatus::parse(s).ok_or_else(|| {
                ApiError::invalid(format!(
                    "{s:?} is not a status (new, matched, added, ignored)"
                ))
            })?);
        }
        Ok(FeedFilter {
            account_id: self.account_id,
            connection_id: self.connection_id,
            statuses,
            from: self.from,
            to: self.to,
            text: self.text,
            amount: self.amount.map(Money),
            limit: self.limit,
            offset: self.offset,
            descending: self.descending,
        })
    }
}

async fn list_txns(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<TxnQuery>,
) -> ApiResult<Json<Vec<FeedTxn>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let filter = q.filter()?;
    Ok(Json(read(books, move |c| queue::query(c, &filter)).await?))
}

async fn get_txn(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedTxnId)>,
) -> ApiResult<Json<FeedTxn>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(read(books, move |c| queue::get(c, id)).await?))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CandidateQuery {
    /// Days either side of the bank's date (default: the connection's, else 10).
    days: Option<u32>,
}

async fn candidates(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedTxnId)>,
    Query(q): Query<CandidateQuery>,
) -> ApiResult<Json<Vec<MatchCandidate>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        read(books, move |c| {
            let f = queue::get(c, id)?;
            let days = match q.days {
                Some(d) => d,
                None => match f.connection_id {
                    Some(conn) => connections::find(c, conn)?
                        .map_or(MatchOptions::default().days, |x| x.match_days),
                    None => MatchOptions::default().days,
                },
            };
            matching::candidates(c, &f, MatchOptions { days: days.min(60) })
        })
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct MatchBody {
    txn_id: TxnId,
}

async fn match_txn(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedTxnId)>,
    Json(b): Json<MatchBody>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    write(books, &p, move |w| matching::accept(w, id, b.txn_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What adding would make: the transaction and its postings, without saving.
#[derive(Debug, Serialize)]
struct Preview {
    txn: Txn,
    postings: Vec<lb_core::Posting>,
    total: Money,
    sales_tax: Money,
}

async fn preview(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedTxnId)>,
    Json(req): Json<AddRequest>,
) -> ApiResult<Json<Preview>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(
        read(books, move |c| {
            let f = queue::get(c, id)?;
            let txn = lb_feeds::add::plan(c, &f, &req)?;
            let posted = lb_core::post(&txn, &StoreContext::new(c)).map_err(StoreError::from)?;
            Ok(Preview {
                txn,
                postings: posted.postings,
                total: posted.total,
                sales_tax: posted.sales_tax,
            })
        })
        .await?,
    ))
}

async fn add(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedTxnId)>,
    body: Option<Json<AddRequest>>,
) -> ApiResult<(StatusCode, Json<AddOutcome>)> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let out = write(books, &p, move |w| lb_feeds::add::add(w, id, &req)).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Debug, Deserialize)]
struct AddAllBody {
    ids: Vec<FeedTxnId>,
    #[serde(default)]
    request: Option<AddRequest>,
}

async fn add_all(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(b): Json<AddAllBody>,
) -> ApiResult<Json<BatchOutcome>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    Ok(Json(
        write(books, &p, move |w| {
            lb_feeds::add::add_all(w, &b.ids, b.request.as_ref())
        })
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct IdsBody {
    ids: Vec<FeedTxnId>,
}

async fn ignore(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(b): Json<IdsBody>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let n = write(books, &p, move |w| queue::ignore(w, &b.ids)).await?;
    Ok(Json(json!({ "ignored": n })))
}

async fn undo(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedTxnId)>,
) -> ApiResult<Json<FeedTxn>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    Ok(Json(write(books, &p, move |w| queue::undo(w, id)).await?))
}

// ---------------------------------------------------------------------------
// Files

/// A multipart upload: the `file` part and the text parts, by name.
struct Upload {
    file: Option<Vec<u8>>,
    fields: std::collections::HashMap<String, String>,
}

impl Upload {
    async fn read(mut form: Multipart) -> ApiResult<Upload> {
        let mut out = Upload {
            file: None,
            fields: std::collections::HashMap::new(),
        };
        while let Some(field) = form
            .next_field()
            .await
            .map_err(|e| ApiError::invalid(e.to_string()))?
        {
            let Some(name) = field.name().map(str::to_owned) else {
                continue;
            };
            if name == "file" {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::invalid(e.to_string()))?;
                out.file = Some(bytes.to_vec());
            } else {
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiError::invalid(e.to_string()))?;
                out.fields.insert(name, text);
            }
        }
        Ok(out)
    }

    fn file(&mut self) -> ApiResult<Vec<u8>> {
        self.file
            .take()
            .filter(|f| !f.is_empty())
            .ok_or_else(|| ApiError::invalid("no file was sent"))
    }

    fn field<T: std::str::FromStr>(&self, name: &str) -> ApiResult<Option<T>> {
        match self
            .fields
            .get(name)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            None => Ok(None),
            Some(v) => v
                .parse()
                .map(Some)
                .map_err(|_| ApiError::invalid(format!("{name} {v:?} is not valid"))),
        }
    }

    fn json<T: serde::de::DeserializeOwned>(&self, name: &str) -> ApiResult<Option<T>> {
        match self
            .fields
            .get(name)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            None => Ok(None),
            Some(v) => serde_json::from_str(v)
                .map(Some)
                .map_err(|e| ApiError::invalid(format!("{name}: {e}"))),
        }
    }
}

/// What `POST /feeds/detect` found.
#[derive(Debug, Serialize)]
struct Detected {
    kind: FileKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    csv: Option<CsvDetection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    statements: Option<Vec<Statement>>,
}

async fn detect(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    form: Multipart,
) -> ApiResult<Json<Detected>> {
    s.books_for(&p, company, Role::Viewer)?;
    let mut up = Upload::read(form).await?;
    let bytes = up.file()?;
    let out = fblocking(move || {
        Ok(match lb_feeds::sniff(&bytes) {
            FileKind::Ofx => Detected {
                kind: FileKind::Ofx,
                csv: None,
                statements: Some(ofx::parse_statements(&bytes)?),
            },
            FileKind::Csv => Detected {
                kind: FileKind::Csv,
                csv: Some(csvfile::detect(&bytes)?),
                statements: None,
            },
        })
    })
    .await?;
    Ok(Json(out))
}

/// `POST /feeds/import` (multipart): `file`, and `account_id`, `connection_id`,
/// `mapping` (a `CsvMapping` as JSON), `statement` (which one, for a file holding
/// several accounts) as text parts.
async fn import(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    form: Multipart,
) -> ApiResult<Json<IngestReport>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let mut up = Upload::read(form).await?;
    let bytes = up.file()?;
    let account: Option<AccountId> = up.field("account_id")?;
    let connection: Option<FeedConnectionId> = up.field("connection_id")?;
    let which: Option<usize> = up.field("statement")?;
    let mapping: Option<CsvMapping> = up.json("mapping")?;
    let report = write(books, &p, move |w| {
        let conn = match connection {
            Some(id) => Some(connections::get(&w.tx, id)?),
            None => None,
        };
        let mut statements = match lb_feeds::sniff(&bytes) {
            FileKind::Ofx => ofx::parse_statements(&bytes)?,
            FileKind::Csv => {
                let saved = conn.as_ref().and_then(|c| {
                    if let ProviderSettings::File { csv_mapping } = &c.settings {
                        csv_mapping.clone()
                    } else {
                        None
                    }
                });
                let mapping = if let Some(m) = mapping.or(saved) {
                    m
                } else {
                        let d = csvfile::detect(&bytes)?;
                    if !d.confident {
                        return Err(FeedError::Invalid(
                            "the columns of this CSV file were guessed; check the mapping (POST /feeds/detect) and send it with the import".into(),
                        ));
                    }
                    d.mapping
                };
                vec![csvfile::parse(&bytes, &mapping)?]
            }
        };
        if let Some(i) = which {
            if i >= statements.len() {
                return Err(FeedError::Invalid(format!(
                    "the file holds {} account(s); statement {i} isn't one",
                    statements.len()
                )));
            }
            statements = vec![statements.swap_remove(i)];
        }
        // A file from an account imported before goes where it went last time.
        let conn = match (conn, account, statements.as_slice()) {
            (Some(c), _, _) => Some(c),
            (None, None, [only]) if !only.account.account_key.is_empty() => {
                connections::by_account_key(&w.tx, &only.account.account_key)?
            }
            _ => None,
        };
        if let Some(conn) = conn {
            let mut conn = conn;
            if conn.account_id.is_none() {
                conn.account_id = account;
            }
            return lb_feeds::ingest::ingest_for_connection(w, &conn, &statements);
        }
        let account = account.ok_or_else(|| {
            FeedError::Invalid("choose the account these transactions belong to".into())
        })?;
        let [statement] = statements.as_slice() else {
            return Err(FeedError::Invalid(format!(
                "the file holds {} accounts; choose one (statement) or import it through a connection",
                statements.len()
            )));
        };
        let opts = IngestOptions {
            import: ImportOptions {
                source: "file".into(),
                ..ImportOptions::default()
            },
            ..IngestOptions::default()
        };
        lb_feeds::ingest::ingest(w, account, statement, &opts)
    })
    .await?;
    Ok(Json(report))
}

// ---------------------------------------------------------------------------
// Connections

/// Every credential reference in a connection's settings.
fn credential_refs(settings: &ProviderSettings) -> Vec<&lb_feeds::SecretRef> {
    match settings {
        ProviderSettings::File { .. } => Vec::new(),
        ProviderSettings::SimpleFin { access, .. } => vec![access],
        ProviderSettings::OfxDirect(o) => vec![&o.password],
        ProviderSettings::Plaid { access_token, .. }
        | ProviderSettings::Teller { access_token, .. } => vec![access_token],
    }
}

/// Refuse references outside the company's own namespace.
fn check_refs(secrets: &CompanySecrets, conn: &FeedConnection) -> ApiResult<()> {
    for r in credential_refs(&conn.settings) {
        if !r.0.is_empty() && !secrets.owns(&r.0) {
            return Err(ApiError::invalid(
                "credentials are set through LibreBooks (the SimpleFIN claim, or PUT …/credentials), not as references",
            )
            .with_detail(json!({ "reference": r.0 })));
        }
    }
    Ok(())
}

async fn list_connections(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Vec<FeedConnection>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(read(books, connections::list).await?))
}

async fn get_connection(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedConnectionId)>,
) -> ApiResult<Json<FeedConnection>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(read(books, move |c| connections::get(c, id)).await?))
}

async fn create_connection(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(conn): Json<FeedConnection>,
) -> ApiResult<(StatusCode, Json<FeedConnection>)> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    check_refs(&s.ops.secrets.for_company(company), &conn)?;
    if conn.edit_seq != 0 {
        return Err(ApiError::invalid("a new connection has edit_seq 0"));
    }
    let saved = write(books, &p, move |w| connections::save(w, conn)).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn update_connection(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedConnectionId)>,
    Json(mut conn): Json<FeedConnection>,
) -> ApiResult<Json<FeedConnection>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    check_refs(&s.ops.secrets.for_company(company), &conn)?;
    conn.id = id;
    Ok(Json(
        write(books, &p, move |w| {
            if conn.edit_seq == 0 {
                return Err(FeedError::Invalid(
                    "send the edit_seq the connection was read with".into(),
                ));
            }
            connections::save(w, conn)
        })
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct EditSeq {
    edit_seq: i64,
}

async fn delete_connection(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedConnectionId)>,
    Query(q): Query<EditSeq>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let (gone, others) = write(books, &p, move |w| {
        let conn = connections::get(&w.tx, id)?;
        connections::delete(w, id, q.edit_seq)?;
        let others = connections::list(&w.tx)?;
        Ok((conn, others))
    })
    .await?;
    // Its credentials go too, unless another connection shares them.
    let mine = credential_refs(&gone.settings)
        .into_iter()
        .map(|r| r.0.clone())
        .collect();
    forget_unused(s.ops.secrets.for_company(company), mine, &others).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct CredentialsBody {
    /// The password, PIN or token.
    value: String,
}

impl std::fmt::Debug for CredentialsBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialsBody([redacted])")
    }
}

/// `PUT /feeds/connections/:id/credentials` `{ value }`: save the connection's
/// password (Direct Connect), access URL (SimpleFIN) or token (Plaid, Teller) in
/// the secret store and point the connection at it. Returns the connection.
async fn set_credentials(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedConnectionId)>,
    Json(body): Json<CredentialsBody>,
) -> ApiResult<Json<FeedConnection>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    if body.value.trim().is_empty() {
        return Err(ApiError::invalid("the credential is empty"));
    }
    let secrets = s.ops.secrets.for_company(company);
    let conn = read(books.clone(), move |c| connections::get(c, id)).await?;
    if matches!(conn.settings, ProviderSettings::File { .. }) {
        return Err(ApiError::invalid("a file connection has no credentials"));
    }
    let store = secrets.clone();
    let reference =
        tokio::task::spawn_blocking(move || store.set(&id.to_string(), &body.value)).await??;
    let previous: Vec<String> = credential_refs(&conn.settings)
        .into_iter()
        .map(|r| r.0.clone())
        .filter(|r| *r != reference.0)
        .collect();
    let mut conn = conn;
    match &mut conn.settings {
        ProviderSettings::SimpleFin { access, .. } => *access = reference,
        ProviderSettings::OfxDirect(o) => o.password = reference,
        ProviderSettings::Plaid { access_token, .. }
        | ProviderSettings::Teller { access_token, .. } => *access_token = reference,
        ProviderSettings::File { .. } => {}
    }
    let (saved, all) = write(books, &p, move |w| {
        let saved = connections::save(w, conn)?;
        Ok((saved, connections::list(&w.tx)?))
    })
    .await?;
    forget_unused(secrets, previous, &all).await?;
    Ok(Json(saved))
}

/// Delete credentials in `candidates` that no connection in `remaining` uses
/// any more (one SimpleFIN access URL can serve several connections).
async fn forget_unused(
    secrets: CompanySecrets,
    candidates: Vec<String>,
    remaining: &[FeedConnection],
) -> ApiResult<()> {
    let used: Vec<String> = remaining
        .iter()
        .flat_map(|c| credential_refs(&c.settings))
        .map(|r| r.0.clone())
        .collect();
    let orphans: Vec<String> = candidates
        .into_iter()
        .filter(|r| !used.contains(r))
        .collect();
    tokio::task::spawn_blocking(move || {
        for r in orphans {
            if let Err(e) = secrets.delete(&r) {
                tracing::warn!(error = %e, "could not delete a feed credential");
            }
        }
    })
    .await?;
    Ok(())
}

async fn sync(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedConnectionId)>,
) -> ApiResult<Json<IngestReport>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    Ok(Json(
        crate::ops::feeds::sync_connection(&s, company, &books, id, &p.actor()).await?,
    ))
}

async fn remote_accounts(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedConnectionId)>,
) -> ApiResult<Json<Vec<StatementAccount>>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let conn = read(books, move |c| connections::get(c, id)).await?;
    let secrets = s.ops.secrets.for_company(company);
    let client = providers::http_client()?;
    let accounts = match &conn.settings {
        ProviderSettings::OfxDirect(o) => {
            let password = lb_feeds::secrets::resolve(&secrets, &o.password, "the bank")?;
            providers::ofx_direct::OfxDirect::new(client, o.clone(), password)?
                .accounts()
                .await?
        }
        ProviderSettings::SimpleFin { access, .. } => {
            let access = lb_feeds::secrets::resolve(&secrets, access, "SimpleFIN")?;
            simplefin_accounts(client, &access).await?.0
        }
        other => {
            return Err(
                FeedError::Unsupported(format!("listing accounts for {}", other.kind())).into(),
            );
        }
    };
    Ok(Json(accounts))
}

/// The bridge's accounts (with no transactions to speak of) and its messages.
async fn simplefin_accounts(
    client: reqwest::Client,
    access: &lb_feeds::Secret,
) -> lb_feeds::Result<(Vec<StatementAccount>, Vec<String>)> {
    let sf = providers::simplefin::SimpleFin::new(client, access)?;
    let download = sf.accounts(Some(jiff::Zoned::now().date())).await?;
    Ok((
        download
            .statements
            .into_iter()
            .map(|st| st.account)
            .collect(),
        download.errors,
    ))
}

#[derive(Deserialize)]
struct ClaimBody {
    setup_token: String,
}

impl std::fmt::Debug for ClaimBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClaimBody([redacted])")
    }
}

/// `POST /feeds/simplefin/claim` `{ setup_token }`: trade the one-time setup
/// token for an access URL (saved as this company's secret), and list the
/// bridge's accounts so the person can connect each to a register.
async fn simplefin_claim(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(body): Json<ClaimBody>,
) -> ApiResult<Json<Value>> {
    s.books_for(&p, company, Role::Bookkeeper)?;
    let client = providers::http_client()?;
    let access = providers::simplefin::claim(&client, body.setup_token.trim()).await?;
    let (accounts, errors) = simplefin_accounts(client, &access).await?;
    let secrets = s.ops.secrets.for_company(company);
    let suffix = format!("simplefin-{}", uuid::Uuid::now_v7().simple());
    let value = access.expose().to_owned();
    let reference = tokio::task::spawn_blocking(move || secrets.set(&suffix, &value)).await??;
    Ok(Json(json!({
        "access": reference,
        "accounts": accounts,
        "errors": errors,
    })))
}

// ---------------------------------------------------------------------------
// Rules

async fn list_rules(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Vec<FeedRule>>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(read(books, rules::list).await?))
}

async fn get_rule(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedRuleId)>,
) -> ApiResult<Json<FeedRule>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(read(books, move |c| rules::get(c, id)).await?))
}

async fn create_rule(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(rule): Json<FeedRule>,
) -> ApiResult<(StatusCode, Json<FeedRule>)> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let saved = write(books, &p, move |w| rules::save(w, rule)).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn update_rule(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedRuleId)>,
    Json(mut rule): Json<FeedRule>,
) -> ApiResult<Json<FeedRule>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    rule.id = id;
    Ok(Json(write(books, &p, move |w| rules::save(w, rule)).await?))
}

async fn delete_rule(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, FeedRuleId)>,
) -> ApiResult<StatusCode> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    write(books, &p, move |w| rules::delete(w, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct ReorderBody {
    ids: Vec<FeedRuleId>,
}

async fn reorder_rules(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(b): Json<ReorderBody>,
) -> ApiResult<Json<Vec<FeedRule>>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    Ok(Json(
        write(books, &p, move |w| {
            rules::reorder(w, &b.ids)?;
            rules::list(&w.tx)
        })
        .await?,
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ApplyBody {
    /// These transactions; or every waiting one (in `account_id`, if given).
    ids: Option<Vec<FeedTxnId>>,
    account_id: Option<AccountId>,
}

async fn apply_rules(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    body: Option<Json<ApplyBody>>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let b = body.map(|b| b.0).unwrap_or_default();
    let n = write(books, &p, move |w| match b.ids {
        Some(ids) => rules::apply_to(w, &ids),
        None => rules::apply_all(w, b.account_id),
    })
    .await?;
    Ok(Json(json!({ "applied": n })))
}

// ---------------------------------------------------------------------------
// Amazon

/// The proposed pairs, and the orders they came from (send both back to
/// `/feeds/amazon/apply`).
#[derive(Debug, Serialize, Deserialize)]
struct AmazonPreview {
    matches: Vec<AmazonMatch>,
    orders: Vec<AmazonOrder>,
}

/// `POST /feeds/amazon/orders` (multipart `file`: Amazon's order-history CSV;
/// optional `account_id` to look only at one card): pair waiting card charges
/// with orders. Changes nothing.
async fn amazon_orders(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    form: Multipart,
) -> ApiResult<Json<AmazonPreview>> {
    let books = s.books_for(&p, company, Role::Viewer)?;
    let mut up = Upload::read(form).await?;
    let bytes = up.file()?;
    let account: Option<AccountId> = up.field("account_id")?;
    Ok(Json(
        read(books, move |c| {
            let orders = amazon::parse_order_history(&bytes)?;
            let charges = queue::query(
                c,
                &FeedFilter {
                    account_id: account,
                    statuses: vec![FeedStatus::New],
                    ..FeedFilter::default()
                },
            )?;
            let matches = amazon::match_charges(&charges, &orders);
            Ok(AmazonPreview { matches, orders })
        })
        .await?,
    ))
}

async fn amazon_apply(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(b): Json<AmazonPreview>,
) -> ApiResult<Json<Value>> {
    let books = s.books_for(&p, company, Role::Bookkeeper)?;
    let n = write(books, &p, move |w| {
        amazon::annotate(w, &b.matches, &b.orders)
    })
    .await?;
    Ok(Json(json!({ "annotated": n })))
}
