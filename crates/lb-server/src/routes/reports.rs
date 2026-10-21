//! Reports: the catalog, running one, and exporting it as CSV, XLSX or PDF.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::Response;
use lb_core::CompanyId;
use lb_reports::{
    CatalogEntry, ExportError, ExportFormat, ExportOptions, Paper, Report, ReportError,
    ReportRequest,
};
use serde::Deserialize;

use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

impl From<ReportError> for ApiError {
    fn from(e: ReportError) -> Self {
        match e {
            ReportError::Store(s) => s.into(),
            ReportError::Invalid(m) => ApiError::invalid(m),
        }
    }
}

impl From<ExportError> for ApiError {
    fn from(e: ExportError) -> Self {
        ApiError::internal(e.to_string())
    }
}

pub async fn catalog(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
) -> ApiResult<Json<Vec<CatalogEntry>>> {
    s.books_for(&p, company, Role::Viewer)?;
    Ok(Json(lb_reports::catalog()))
}

async fn run(
    s: &AppState,
    p: &Principal,
    company: CompanyId,
    req: ReportRequest,
) -> ApiResult<Report> {
    let books = s.books_for(p, company, Role::Viewer)?;
    Ok(tokio::task::spawn_blocking(move || lb_reports::run(&books, &req)).await??)
}

pub async fn run_report(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(req): Json<ReportRequest>,
) -> ApiResult<Json<Report>> {
    Ok(Json(run(&s, &p, company, req).await?))
}

#[derive(Deserialize)]
pub struct ExportQuery {
    /// `csv` (default), `xlsx` or `pdf`.
    #[serde(default)]
    format: Option<String>,
    /// PDF only: `letter` (default) or `a4`.
    #[serde(default)]
    paper: Option<String>,
}

pub async fn export(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<ExportQuery>,
    Json(req): Json<ReportRequest>,
) -> ApiResult<Response> {
    let format = match q.format.as_deref() {
        None => ExportFormat::Csv,
        Some(f) => ExportFormat::parse(f).ok_or_else(|| {
            ApiError::invalid(format!("reports export as csv, xlsx or pdf, not {f}"))
        })?,
    };
    let paper = match q.paper.as_deref() {
        None => Paper::default(),
        Some(p) => p.parse::<Paper>().map_err(ApiError::invalid)?,
    };
    let report = run(&s, &p, company, req).await?;
    let opts = ExportOptions { paper };
    let (bytes, name) = tokio::task::spawn_blocking(move || {
        let bytes = lb_reports::export_with(&report, format, &opts)?;
        Ok::<_, ExportError>((bytes, lb_reports::file_name(&report, format)))
    })
    .await??;
    let name = name.replace(['"', '\\'], "_");
    Response::builder()
        .header(header::CONTENT_TYPE, format.mime())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(Body::from(bytes))
        .map_err(|e| ApiError::internal(e.to_string()))
}
