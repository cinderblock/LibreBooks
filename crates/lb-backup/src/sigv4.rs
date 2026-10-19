//! AWS Signature Version 4, for S3-compatible storage.
//!
//! Implemented here rather than pulled in with an AWS SDK: the SDKs are large, and
//! the S3 clients that are small either drag in OpenSSL or are thinly maintained.
//! SigV4 itself is a page of hashing, and it is checked below against AWS's own
//! published examples (the aws-c-auth signing test suite and the S3 documentation),
//! so a mistake here cannot hide.
//!
//! Paths follow S3's rule: each segment is URI-encoded once (other AWS services
//! encode twice and normalise `.` and `..`; S3 does neither, and neither does this).

use hmac::{Hmac, KeyInit, Mac};
use jiff::Timestamp;
use sha2::{Digest, Sha256};

/// SHA-256 of nothing, the payload hash of a request without a body.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Access key pair.
#[derive(Clone, Copy)]
pub struct Credentials<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
}

impl std::fmt::Debug for Credentials<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .finish_non_exhaustive()
    }
}

/// The parts of a request that are signed.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: &'a str,
    /// The path as it should be read, not yet encoded: `/bucket/my file.txt`.
    pub path: &'a str,
    /// Query parameters, not yet encoded. A flag such as `?lifecycle` is
    /// `("lifecycle", "")`.
    pub query: &'a [(String, String)],
    /// Every header to sign, including `host` and `x-amz-date`. Names in any case.
    pub headers: &'a [(String, String)],
    /// Hex SHA-256 of the body (or `UNSIGNED-PAYLOAD`).
    pub payload_sha256: &'a str,
}

/// A signature and the intermediate strings, which are what to compare when a
/// server says the signature does not match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub canonical_request: String,
    pub string_to_sign: String,
    pub signature: String,
    /// The `Authorization` header's value.
    pub authorization: String,
}

/// Sign `request` for `service` (`s3`) in `region` at time `at`. `at` must match
/// the request's `x-amz-date` header ([`amz_date`]).
pub fn sign(
    credentials: Credentials<'_>,
    region: &str,
    service: &str,
    at: Timestamp,
    request: &Request<'_>,
) -> Signed {
    let (canonical_headers, signed_headers) = canonical_headers(request.headers);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        request.method,
        canonical_path(request.path),
        canonical_query(request.query),
        canonical_headers,
        signed_headers,
        request.payload_sha256,
    );
    let date = at.strftime("%Y%m%d").to_string();
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        amz_date(at),
        hex::encode(Sha256::digest(canonical_request.as_bytes())),
    );
    let secret = format!("AWS4{}", credentials.secret_access_key);
    let k_date = hmac(secret.as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    );
    Signed {
        canonical_request,
        string_to_sign,
        signature,
        authorization,
    }
}

/// `20150830T123600Z`, the form of `x-amz-date`.
pub fn amz_date(at: Timestamp) -> String {
    at.strftime("%Y%m%dT%H%M%SZ").to_string()
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// RFC 3986 percent-encoding of everything but unreserved characters, with `/`
/// kept when `keep_slash`. Uppercase hex, as SigV4 requires.
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        if unreserved || (keep_slash && b == b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn canonical_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else {
        uri_encode(path, true)
    }
}

/// The query as it is signed, and as it must then be sent: encoded pairs sorted by
/// name, then value.
pub fn canonical_query(query: &[(String, String)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn canonical_headers(headers: &[(String, String)]) -> (String, String) {
    // Lowercase names; trimmed values with inner runs of spaces collapsed; repeated
    // headers joined with commas in the order given; sorted by name.
    let mut merged: Vec<(String, String)> = Vec::new();
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        match merged.iter_mut().find(|(n, _)| *n == name) {
            Some((_, v)) => {
                v.push(',');
                v.push_str(&value);
            }
            None => merged.push((name, value)),
        }
    }
    merged.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical = merged.iter().fold(String::new(), |mut out, (n, v)| {
        out.push_str(n);
        out.push(':');
        out.push_str(v);
        out.push('\n');
        out
    });
    let signed = merged
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(";");
    (canonical, signed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// The aws-c-auth signing test suite's credentials and time.
    const SUITE: Credentials<'static> = Credentials {
        access_key_id: "AKIDEXAMPLE",
        secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
    };

    fn suite_time() -> Timestamp {
        "2015-08-30T12:36:00Z".parse().unwrap()
    }

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    fn suite(
        method: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Signed {
        let mut all = h(&[("Host", "example.amazonaws.com")]);
        all.extend(h(headers));
        all.push(("X-Amz-Date".into(), "20150830T123600Z".into()));
        let payload = hex::encode(Sha256::digest(body));
        sign(
            SUITE,
            "us-east-1",
            "service",
            suite_time(),
            &Request {
                method,
                path: "/",
                query: &h(query),
                headers: &all,
                payload_sha256: &payload,
            },
        )
    }

    // Vectors from github.com/awslabs/aws-c-auth, tests/aws-signing-test-suite/v4.

    #[test]
    fn get_vanilla() {
        let s = suite("GET", &[], &[], b"");
        assert_eq!(
            s.canonical_request,
            "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            s.signature,
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        assert_eq!(
            s.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn get_vanilla_query_order_key_case() {
        let s = suite(
            "GET",
            &[("Param2", "value2"), ("Param1", "value1")],
            &[],
            b"",
        );
        assert!(
            s.canonical_request
                .contains("\nParam1=value1&Param2=value2\n")
        );
        assert_eq!(
            s.signature,
            "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    #[test]
    fn get_vanilla_query_unreserved() {
        let u = "-._~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let s = suite("GET", &[(u, u)], &[], b"");
        assert_eq!(
            s.signature,
            "9c3e54bfcdf0b19771a7f523ee5669cdf59bc7cc0884027167c21bb143a40197"
        );
    }

    #[test]
    fn get_vanilla_utf8_query() {
        let s = suite("GET", &[("\u{1234}", "bar")], &[], b"");
        assert!(s.canonical_request.contains("\n%E1%88%B4=bar\n"));
        assert_eq!(
            s.signature,
            "2cdec8eed098649ff3a119c94853b13c643bcf08f8b0a1d91e12c9027818dd04"
        );
    }

    #[test]
    fn get_header_value_trim() {
        let s = suite(
            "GET",
            &[],
            &[("My-Header1", " value1"), ("My-Header2", " \"a   b   c\"")],
            b"",
        );
        assert!(s.canonical_request.contains("my-header2:\"a b c\"\n"));
        assert_eq!(
            s.signature,
            "acc3ed3afb60bb290fc8d2dd0098b9911fcaa05412b367055dee359757a9c736"
        );
    }

    #[test]
    fn get_header_key_duplicate() {
        let s = suite(
            "GET",
            &[],
            &[
                ("My-Header1", "value2"),
                ("My-Header1", "value2"),
                ("My-Header1", "value1"),
            ],
            b"",
        );
        assert!(
            s.canonical_request
                .contains("my-header1:value2,value2,value1\n")
        );
        assert_eq!(
            s.signature,
            "c9d5ea9f3f72853aea855b47ea873832890dbdd183b4468f858259531a5138ea"
        );
    }

    #[test]
    fn post_vanilla_query() {
        let s = suite("POST", &[("Param1", "value1")], &[], b"");
        assert_eq!(
            s.signature,
            "28038455d6de14eafc1f9222cf5aa6f1a96197d7deb8263271d420d138af7f11"
        );
    }

    #[test]
    fn post_x_www_form_urlencoded() {
        let body = b"Param1=value1";
        let hash = hex::encode(Sha256::digest(body));
        let s = suite(
            "POST",
            &[],
            &[
                ("Content-Type", "application/x-www-form-urlencoded"),
                ("Content-Length", "13"),
                ("X-Amz-Content-Sha256", &hash),
            ],
            body,
        );
        assert_eq!(
            s.signature,
            "d3875051da38690788ef43de4db0d8f280229d82040bfac253562e56c3f20e0b"
        );
    }

    // The worked examples in the S3 API reference ("Signature Calculations for the
    // Authorization Header: Transferring Payload in a Single Chunk").

    const S3_DOCS: Credentials<'static> = Credentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE",
        secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    };

    fn s3_docs(
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        extra: &[(&str, &str)],
        payload: &str,
    ) -> Signed {
        let mut headers = h(&[("Host", "examplebucket.s3.amazonaws.com")]);
        headers.extend(h(extra));
        headers.push(("x-amz-content-sha256".into(), payload.into()));
        headers.push(("x-amz-date".into(), "20130524T000000Z".into()));
        sign(
            S3_DOCS,
            "us-east-1",
            "s3",
            "2013-05-24T00:00:00Z".parse().unwrap(),
            &Request {
                method,
                path,
                query: &h(query),
                headers: &headers,
                payload_sha256: payload,
            },
        )
    }

    #[test]
    fn s3_get_object() {
        let s = s3_docs(
            "GET",
            "/test.txt",
            &[],
            &[("Range", "bytes=0-9")],
            EMPTY_SHA256,
        );
        assert_eq!(
            s.signature,
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn s3_put_object_encodes_the_key() {
        let payload = hex::encode(Sha256::digest(b"Welcome to Amazon S3."));
        assert_eq!(
            payload,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let s = s3_docs(
            "PUT",
            "/test$file.text",
            &[],
            &[
                ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ],
            &payload,
        );
        assert!(s.canonical_request.starts_with("PUT\n/test%24file.text\n"));
        assert_eq!(
            s.signature,
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn s3_get_bucket_lifecycle_flag_parameter() {
        let s = s3_docs("GET", "/", &[("lifecycle", "")], &[], EMPTY_SHA256);
        assert!(s.canonical_request.contains("\nlifecycle=\n"));
        assert_eq!(
            s.signature,
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
        );
    }

    #[test]
    fn s3_list_objects() {
        let s = s3_docs(
            "GET",
            "/",
            &[("max-keys", "2"), ("prefix", "J")],
            &[],
            EMPTY_SHA256,
        );
        assert_eq!(
            s.signature,
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
    }

    #[test]
    fn encoding_keeps_only_unreserved_characters() {
        assert_eq!(uri_encode("a b/c+d~é", true), "a%20b/c%2Bd~%C3%A9");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(canonical_path(""), "/");
    }
}
