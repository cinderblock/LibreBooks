//! The web UI, embedded in the binary from `web/dist` at build time.
//!
//! Any path that isn't an asset and isn't under `/api` gets `index.html`, so the
//! single-page app's own routes (`/c/<id>/registers/…`) survive a reload. Hashed
//! assets (`/assets/*`) are cached for a year; `index.html` never is, so a new
//! version shows up on the next load.

use axum::body::Body;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Web;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, "no such API route").into_response();
    }
    if !path.is_empty()
        && let Some(file) = Web::get(path)
    {
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return Response::builder()
            .header(header::CONTENT_TYPE, file.metadata.mimetype())
            .header(header::CACHE_CONTROL, cache)
            .body(Body::from(file.data.into_owned()))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    match Web::get("index.html") {
        Some(index) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(index.data.into_owned()))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        None => (
            StatusCode::NOT_FOUND,
            "This build has no web UI embedded (build web/ first: `bun run build`). The API is at /api/v1.",
        )
            .into_response(),
    }
}

/// Whether a web UI is embedded (for `/health` and startup messages).
pub fn has_ui() -> bool {
    Web::get("index.html").is_some()
}
