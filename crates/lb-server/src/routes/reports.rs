//! Reports: the catalog, running one, and exporting it.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::Response;
use lb_core::CompanyId;
use lb_reports::{CatalogEntry, ExportFormat, Report, ReportError, ReportRequest};
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
    #[serde(default)]
    format: Option<String>,
}

pub async fn export(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<ExportQuery>,
    Json(req): Json<ReportRequest>,
) -> ApiResult<Response> {
    let format = match q.format.as_deref().unwrap_or("csv") {
        "csv" => ExportFormat::Csv,
        other => {
            return Err(ApiError::invalid(format!(
                "reports export as csv, not {other}"
            )));
        }
    };
    let report = run(&s, &p, company, req).await?;
    let bytes = lb_reports::export(&report, format);
    let name = lb_reports::file_name(&report, format).replace(['"', '\\'], "_");
    Response::builder()
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(Body::from(bytes))
        .map_err(|e| ApiError::internal(e.to_string()))
}
