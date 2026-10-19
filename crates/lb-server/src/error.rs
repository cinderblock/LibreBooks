//! Errors as the API reports them: `{ "error": { code, message, detail? } }`
//! with the HTTP statuses `plans/api.md` lists.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lb_store::StoreError;
use serde_json::{Value, json};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub detail: Option<Value>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid", message)
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{what} not found"),
        )
    }

    pub fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthenticated", "sign in first")
    }

    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "you don't have permission to do that",
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    #[must_use]
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(code = self.code, message = %self.message, "request failed");
        }
        let mut body = json!({ "code": self.code, "message": self.message });
        if let Some(d) = self.detail {
            body["detail"] = d;
        }
        (self.status, Json(json!({ "error": body }))).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        let message = e.to_string();
        match e {
            StoreError::NotFound { .. } => {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", message)
            }
            StoreError::Conflict {
                expected, actual, ..
            } => ApiError::new(StatusCode::CONFLICT, "conflict", message)
                .with_detail(json!({ "expected": expected, "actual": actual })),
            StoreError::Duplicate { .. } => {
                ApiError::new(StatusCode::CONFLICT, "duplicate", message)
            }
            StoreError::InUse { .. } => ApiError::new(StatusCode::CONFLICT, "in_use", message),
            StoreError::Invalid(_) => ApiError::new(StatusCode::BAD_REQUEST, "invalid", message),
            StoreError::ClosedPeriod { closing_date } => {
                ApiError::new(StatusCode::LOCKED, "closed_period", message)
                    .with_detail(json!({ "closing_date": closing_date }))
            }
            StoreError::Posting(p) => {
                let detail = serde_json::to_value(&p).unwrap_or(Value::Null);
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "posting", message)
                    .with_detail(detail)
            }
            StoreError::TooNew { .. }
            | StoreError::Corrupt(_)
            | StoreError::Sqlite(_)
            | StoreError::Json(_) => ApiError::internal(message),
        }
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        ApiError::internal(format!("a worker failed: {e}"))
    }
}
