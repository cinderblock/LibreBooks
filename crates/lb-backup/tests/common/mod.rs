//! Shared test helpers: a real company file, and a small S3 look-alike.

#![allow(dead_code)]
// Handlers return a ready response as the error; its size does not matter here.
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use lb_backup::sigv4;
use lb_core::{AccountType, CompanyInfo};
use lb_store::{Actor, Books, accounts};
use sha2::{Digest, Sha256};

/// A company file on disk with a couple of accounts in it.
pub fn company(dir: &Path, name: &str) -> (Books, PathBuf) {
    let path = dir.join(format!("{name}.lbdb"));
    let info = CompanyInfo {
        name: name.into(),
        ..CompanyInfo::default()
    };
    let books = Books::create(&path, &info).unwrap();
    books
        .write(&Actor::system(), |w| {
            accounts::save(w, accounts::new_account("Checking", AccountType::Bank))?;
            accounts::save(w, accounts::new_account("Rent", AccountType::Expense))?;
            Ok(())
        })
        .unwrap();
    (books, path)
}

/// The accounts [`company`] added that are in a company file, sorted (a new
/// company also has the store's own system accounts, which are left out).
pub fn account_names(path: &Path) -> Vec<String> {
    let books = Books::open(path).unwrap();
    let mut names: Vec<String> = books
        .read(|c| accounts::list(c, true))
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .filter(|n| n == "Checking" || n == "Rent")
        .collect();
    names.sort();
    names
}

pub const ACCESS_KEY: &str = "AKIDTEST";
pub const SECRET_KEY: &str = "test-secret-key/with+symbols";
pub const BUCKET: &str = "books";

/// An S3 look-alike: one bucket, path-style, objects in memory. It checks every
/// request's SigV4 signature itself, from the request as received, so a client
/// that signs one thing and sends another fails here as it would against AWS.
#[derive(Default)]
pub struct MockS3 {
    pub objects: Mutex<BTreeMap<String, Vec<u8>>>,
    /// Requests seen, as `METHOD path`.
    pub log: Mutex<Vec<String>>,
}

/// Start the mock on a free port; returns its endpoint URL.
pub async fn start_mock_s3() -> (String, Arc<MockS3>) {
    let mock = Arc::new(MockS3::default());
    let app = axum::Router::new()
        .fallback(handle)
        .with_state(Arc::clone(&mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), mock)
}

fn s3_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>{code}</Code><Message>{message}</Message></Error>"
    );
    (status, [("content-type", "application/xml")], body).into_response()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn check_signature(
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), Response> {
    let deny = |m: &str| Err(s3_error(StatusCode::FORBIDDEN, "SignatureDoesNotMatch", m));
    let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) else {
        return deny("no Authorization header");
    };
    let field = |name: &str| {
        auth.split([' ', ','])
            .find_map(|p| p.strip_prefix(&format!("{name}=")))
            .unwrap_or("")
            .to_owned()
    };
    let credential = field("Credential");
    let signed_headers = field("SignedHeaders");
    let signature = field("Signature");
    if !credential.starts_with(&format!("{ACCESS_KEY}/")) {
        return Err(s3_error(
            StatusCode::FORBIDDEN,
            "InvalidAccessKeyId",
            "unknown key",
        ));
    }
    let header = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    let payload = header("x-amz-content-sha256");
    if payload != hex::encode(Sha256::digest(body)) {
        return Err(s3_error(
            StatusCode::BAD_REQUEST,
            "XAmzContentSHA256Mismatch",
            "body hash",
        ));
    }
    let amz_date = header("x-amz-date");
    let at = jiff::civil::DateTime::strptime("%Y%m%dT%H%M%SZ", &amz_date)
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp();
    let signed: Vec<(String, String)> = signed_headers
        .split(';')
        .map(|n| (n.to_owned(), header(n)))
        .collect();
    let query: Vec<(String, String)> = uri
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    let path = percent_decode(uri.path());
    let expected = sigv4::sign(
        sigv4::Credentials {
            access_key_id: ACCESS_KEY,
            secret_access_key: SECRET_KEY,
        },
        "us-east-1",
        "s3",
        at,
        &sigv4::Request {
            method: method.as_str(),
            path: &path,
            query: &query,
            headers: &signed,
            payload_sha256: &payload,
        },
    );
    if expected.signature != signature {
        return deny(&format!(
            "expected canonical request {:?}",
            expected.canonical_request
        ));
    }
    Ok(())
}

async fn handle(
    State(mock): State<Arc<MockS3>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    mock.log
        .lock()
        .unwrap()
        .push(format!("{method} {}", uri.path()));
    if let Err(r) = check_signature(&method, &uri, &headers, &body) {
        return r;
    }
    let path = percent_decode(uri.path());
    let Some(rest) = path.strip_prefix(&format!("/{BUCKET}")) else {
        return s3_error(StatusCode::NOT_FOUND, "NoSuchBucket", "no such bucket");
    };
    let key = rest.trim_start_matches('/').to_owned();
    let mut objects = mock.objects.lock().unwrap();
    match (method, key.is_empty()) {
        (Method::PUT, false) => {
            objects.insert(key, body.to_vec());
            StatusCode::OK.into_response()
        }
        (Method::GET, false) => match objects.get(&key) {
            Some(b) => (StatusCode::OK, b.clone()).into_response(),
            None => s3_error(
                StatusCode::NOT_FOUND,
                "NoSuchKey",
                "The specified key does not exist.",
            ),
        },
        (Method::DELETE, false) => {
            objects.remove(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        (Method::GET, true) => {
            // ListObjectsV2, three keys a page so that paging is exercised.
            let q: BTreeMap<String, String> = uri
                .query()
                .unwrap_or("")
                .split('&')
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (percent_decode(k), percent_decode(v)))
                .collect();
            if q.get("list-type").map(String::as_str) != Some("2") {
                return s3_error(StatusCode::BAD_REQUEST, "InvalidArgument", "list-type");
            }
            let prefix = q.get("prefix").cloned().unwrap_or_default();
            let after = q.get("continuation-token").cloned().unwrap_or_default();
            let keys: Vec<&String> = objects
                .keys()
                .filter(|k| k.starts_with(&prefix) && **k > after)
                .collect();
            let page: Vec<&String> = keys.iter().take(3).copied().collect();
            let truncated = keys.len() > page.len();
            let mut xml = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
            );
            xml.push_str(&format!(
                "<Name>{BUCKET}</Name><Prefix>{prefix}</Prefix><KeyCount>{}</KeyCount>",
                page.len()
            ));
            xml.push_str(&format!("<IsTruncated>{truncated}</IsTruncated>"));
            if truncated {
                xml.push_str(&format!(
                    "<NextContinuationToken>{}</NextContinuationToken>",
                    xml_escape(page.last().unwrap())
                ));
            }
            for k in page {
                xml.push_str(&format!(
                    "<Contents><Key>{}</Key><Size>{}</Size><StorageClass>STANDARD</StorageClass></Contents>",
                    xml_escape(k),
                    objects[k].len()
                ));
            }
            xml.push_str("</ListBucketResult>");
            (StatusCode::OK, [("content-type", "application/xml")], xml).into_response()
        }
        _ => s3_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "MethodNotAllowed",
            "not supported",
        ),
    }
}
