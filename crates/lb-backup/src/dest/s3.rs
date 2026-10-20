//! Any S3-compatible bucket: AWS, Backblaze B2, Cloudflare R2, Wasabi, MinIO, …
//!
//! Five calls are all a backup needs: PutObject, GetObject, ListObjectsV2,
//! DeleteObject, each signed with SigV4 ([`crate::sigv4`]). The body hash is always
//! sent and signed (`x-amz-content-sha256`), so the service itself rejects an
//! upload that was damaged on the way.

use std::path::Path;

use age::secrecy::{ExposeSecret, SecretString};
use jiff::Timestamp;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::config::Destination;
use crate::dest::SMALL_OBJECT_LIMIT;
use crate::secret::SecretResolver;
use crate::sigv4::{self, Credentials, EMPTY_SHA256};
use crate::{BackupError, Result};

/// A bucket (and optional folder in it) with credentials.
pub struct S3Store {
    http: reqwest::Client,
    scheme: String,
    /// `host[:port]` of the endpoint, without the bucket.
    host: String,
    region: String,
    bucket: String,
    /// Empty, or ends with `/`.
    prefix: String,
    path_style: bool,
    access_key_id: String,
    secret_access_key: SecretString,
}

impl std::fmt::Debug for S3Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Store")
            .field("host", &self.host)
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("path_style", &self.path_style)
            .finish_non_exhaustive()
    }
}

impl S3Store {
    pub(crate) fn new(destination: &Destination, secrets: &dyn SecretResolver) -> Result<S3Store> {
        let Destination::S3 {
            endpoint,
            region,
            bucket,
            prefix,
            access_key_id,
            secret_access_key_ref,
            path_style,
        } = destination
        else {
            unreachable!("S3Store::new is only called for S3 destinations");
        };
        if bucket.is_empty() {
            return Err(BackupError::Config(
                "an S3 destination needs a bucket name".into(),
            ));
        }
        let endpoint = if endpoint.trim().is_empty() {
            format!("https://s3.{region}.amazonaws.com")
        } else {
            endpoint.trim().to_owned()
        };
        let url = url::Url::parse(&endpoint)
            .map_err(|e| BackupError::Config(format!("the S3 endpoint {endpoint:?}: {e}")))?;
        let host = match (url.host_str(), url.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_owned(),
            (None, _) => {
                return Err(BackupError::Config(format!(
                    "the S3 endpoint {endpoint:?} has no host"
                )));
            }
        };
        let mut prefix = prefix.trim_matches('/').to_owned();
        if !prefix.is_empty() {
            prefix.push('/');
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("LibreBooks/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(S3Store {
            http,
            scheme: url.scheme().to_owned(),
            host,
            region: region.clone(),
            bucket: bucket.clone(),
            prefix,
            path_style: *path_style,
            access_key_id: access_key_id.clone(),
            secret_access_key: secrets.resolve(secret_access_key_ref)?,
        })
    }

    /// The request's host and path (not yet encoded) for an object key, or for the
    /// bucket itself when `key` is `None`.
    fn location(&self, key: Option<&str>) -> (String, String) {
        let key = key.unwrap_or("");
        if self.path_style {
            let path = if key.is_empty() {
                format!("/{}", self.bucket)
            } else {
                format!("/{}/{key}", self.bucket)
            };
            (self.host.clone(), path)
        } else {
            (format!("{}.{}", self.bucket, self.host), format!("/{key}"))
        }
    }

    /// Send a signed request.
    async fn send(
        &self,
        method: Method,
        key: Option<&str>,
        query: &[(String, String)],
        body: Option<(reqwest::Body, String, u64)>,
    ) -> Result<reqwest::Response> {
        let (host, path) = self.location(key);
        let now = Timestamp::now();
        let (body, payload_hash, length) = match body {
            Some((b, hash, len)) => (Some(b), hash, Some(len)),
            None => (None, EMPTY_SHA256.to_owned(), None),
        };
        let mut headers = vec![
            ("host".to_owned(), host.clone()),
            ("x-amz-content-sha256".to_owned(), payload_hash.clone()),
            ("x-amz-date".to_owned(), sigv4::amz_date(now)),
        ];
        let signed = sigv4::sign(
            Credentials {
                access_key_id: &self.access_key_id,
                secret_access_key: self.secret_access_key.expose_secret(),
            },
            &self.region,
            "s3",
            now,
            &sigv4::Request {
                method: method.as_str(),
                path: &path,
                query,
                headers: &headers,
                payload_sha256: &payload_hash,
            },
        );
        headers.push(("authorization".to_owned(), signed.authorization));
        // Sent exactly as signed: the same encoding and the same parameter order.
        let mut url = format!("{}://{host}{}", self.scheme, sigv4::uri_encode(&path, true));
        let q = sigv4::canonical_query(query);
        if !q.is_empty() {
            url.push('?');
            url.push_str(&q);
        }
        let mut req = self.http.request(method, &url);
        for (name, value) in headers.iter().filter(|(n, _)| n != "host") {
            req = req.header(name, value);
        }
        if let Some(len) = length {
            // S3 refuses chunked uploads, so a streamed body needs its length.
            req = req.header(reqwest::header::CONTENT_LENGTH, len);
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        Ok(req.send().await?)
    }

    pub(crate) async fn put_file(
        &self,
        name: &str,
        path: &Path,
        sha256: &str,
        size: u64,
    ) -> Result<()> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(BackupError::io(format!("open {}", path.display())))?;
        let key = format!("{}{name}", self.prefix);
        let resp = self
            .send(
                Method::PUT,
                Some(&key),
                &[],
                Some((reqwest::Body::from(file), sha256.to_owned(), size)),
            )
            .await?;
        expect_success(resp).await.map(drop)
    }

    pub(crate) async fn put_bytes(&self, name: &str, bytes: Vec<u8>) -> Result<()> {
        let hash = hex::encode(Sha256::digest(&bytes));
        let len = bytes.len() as u64;
        let key = format!("{}{name}", self.prefix);
        let resp = self
            .send(
                Method::PUT,
                Some(&key),
                &[],
                Some((reqwest::Body::from(bytes), hash, len)),
            )
            .await?;
        expect_success(resp).await.map(drop)
    }

    async fn get(&self, name: &str) -> Result<reqwest::Response> {
        let key = format!("{}{name}", self.prefix);
        let resp = self.send(Method::GET, Some(&key), &[], None).await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Err(BackupError::NotFound(format!("s3://{}/{key}", self.bucket)));
        }
        expect_success(resp).await
    }

    pub(crate) async fn get_file(&self, name: &str, dest: &Path) -> Result<()> {
        let mut resp = self.get(name).await?;
        let what = format!("write {}", dest.display());
        let mut out = tokio::fs::File::create(dest)
            .await
            .map_err(BackupError::io(what.clone()))?;
        while let Some(chunk) = resp.chunk().await? {
            out.write_all(&chunk)
                .await
                .map_err(BackupError::io(what.clone()))?;
        }
        out.flush().await.map_err(BackupError::io(what))?;
        Ok(())
    }

    pub(crate) async fn get_bytes(&self, name: &str) -> Result<Vec<u8>> {
        let resp = self.get(name).await?;
        read_limited(resp, SMALL_OBJECT_LIMIT).await
    }

    pub(crate) async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let full_prefix = format!("{}{prefix}", self.prefix);
        let mut names = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![
                ("list-type".to_owned(), "2".to_owned()),
                ("prefix".to_owned(), full_prefix.clone()),
            ];
            if let Some(t) = &token {
                query.push(("continuation-token".to_owned(), t.clone()));
            }
            let resp = self.send(Method::GET, None, &query, None).await?;
            let resp = expect_success(resp).await?;
            let body = read_limited(resp, 64 << 20).await?;
            let page: ListBucketResult =
                quick_xml::de::from_reader(body.as_slice()).map_err(|e| BackupError::Remote {
                    status: 200,
                    message: format!("an unreadable bucket listing ({e})"),
                })?;
            for c in page.contents {
                if let Some(rel) = c.key.strip_prefix(&self.prefix) {
                    names.push(rel.to_owned());
                }
            }
            match page.next_continuation_token {
                Some(t) if page.is_truncated => token = Some(t),
                _ => break,
            }
        }
        Ok(names)
    }

    pub(crate) async fn delete(&self, name: &str) -> Result<()> {
        let key = format!("{}{name}", self.prefix);
        let resp = self.send(Method::DELETE, Some(&key), &[], None).await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        expect_success(resp).await.map(drop)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListBucketResult {
    #[serde(default)]
    contents: Vec<Contents>,
    #[serde(default)]
    is_truncated: bool,
    next_continuation_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Contents {
    key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ErrorBody {
    code: Option<String>,
    message: Option<String>,
}

/// The response if it succeeded; otherwise the service's error, in words.
async fn expect_success(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = read_limited(resp, 64 << 10).await.unwrap_or_default();
    let parsed: Option<ErrorBody> = quick_xml::de::from_reader(body.as_slice()).ok();
    let message = match parsed {
        Some(ErrorBody {
            code: Some(code),
            message,
        }) => match message {
            Some(m) => format!("{code}: {m}"),
            None => code,
        },
        _ => status
            .canonical_reason()
            .unwrap_or("an unexpected error")
            .to_owned(),
    };
    Err(BackupError::Remote {
        status: status.as_u16(),
        message,
    })
}

async fn read_limited(mut resp: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        out.extend_from_slice(&chunk);
        if out.len() as u64 > limit {
            return Err(BackupError::Remote {
                status: resp.status().as_u16(),
                message: "the response was far larger than expected".into(),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::{EnvAndFiles, SecretRef};

    fn store(endpoint: &str, path_style: bool, prefix: &str) -> S3Store {
        let d = Destination::S3 {
            endpoint: endpoint.into(),
            region: "us-east-1".into(),
            bucket: "books".into(),
            prefix: prefix.into(),
            access_key_id: "AKID".into(),
            secret_access_key_ref: SecretRef::new("env:PATH"),
            path_style,
        };
        S3Store::new(&d, &EnvAndFiles).unwrap()
    }

    #[test]
    fn addressing_styles() {
        let s = store("http://127.0.0.1:9000", true, "/lb/");
        assert_eq!(s.prefix, "lb/");
        assert_eq!(
            s.location(Some("lb/a b")),
            ("127.0.0.1:9000".to_owned(), "/books/lb/a b".to_owned())
        );
        assert_eq!(s.location(None).1, "/books");
        let s = store("https://s3.us-west-002.backblazeb2.com", false, "");
        assert_eq!(
            s.location(Some("k")),
            (
                "books.s3.us-west-002.backblazeb2.com".to_owned(),
                "/k".to_owned()
            )
        );
        let s = store("", false, "");
        assert_eq!(s.host, "s3.us-east-1.amazonaws.com");
        assert_eq!(s.scheme, "https");
    }

    #[test]
    fn listings_and_errors_parse() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>books</Name><Prefix>lb/</Prefix><KeyCount>2</KeyCount>
  <IsTruncated>true</IsTruncated>
  <NextContinuationToken>abc</NextContinuationToken>
  <Contents><Key>lb/a</Key><Size>1</Size></Contents>
  <Contents><Key>lb/b</Key><Size>2</Size></Contents>
</ListBucketResult>"#;
        let page: ListBucketResult = quick_xml::de::from_str(xml).unwrap();
        assert_eq!(page.contents.len(), 2);
        assert!(page.is_truncated);
        assert_eq!(page.next_continuation_token.as_deref(), Some("abc"));
        let empty: ListBucketResult = quick_xml::de::from_str(
            "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>",
        )
        .unwrap();
        assert!(empty.contents.is_empty());
        let err: ErrorBody = quick_xml::de::from_str(
            "<Error><Code>SignatureDoesNotMatch</Code><Message>nope</Message></Error>",
        )
        .unwrap();
        assert_eq!(err.code.as_deref(), Some("SignatureDoesNotMatch"));
    }
}
