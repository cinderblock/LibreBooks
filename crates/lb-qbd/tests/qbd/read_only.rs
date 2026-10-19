//! The read-only guarantee, checked on the source itself: the user's live
//! QuickBooks must never receive anything but GET from LibreBooks. Only
//! `src/client.rs` may talk HTTP, and the only request it can build is a GET.
//! If this test fails, someone added a way to write to QuickBooks: don't loosen
//! the test, take the write out.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn only_the_client_talks_http_and_it_can_only_get() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 5, "found {files:?}");
    let mut saw_client = false;
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        if f.file_name().is_some_and(|n| n == "client.rs") {
            saw_client = true;
            // Ignore layout so a reformat can't hide a call.
            let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            for forbidden in [
                ".post(",
                ".put(",
                ".patch(",
                ".delete(",
                ".head(",
                ".request(",
                "Method::",
                "RequestBuilder::new",
                "Request::new",
            ] {
                assert!(
                    !compact.contains(forbidden),
                    "client.rs contains {forbidden:?}: LibreBooks must never write to QuickBooks"
                );
            }
            let uses = compact.matches("self.http.").count();
            let gets = compact.matches("self.http.get(").count();
            assert_eq!(
                uses, gets,
                "every use of the HTTP client in client.rs must be a GET"
            );
            assert_eq!(gets, 1, "one place sends requests");
        } else {
            for http in ["reqwest", "hyper", "TcpStream", "ureq", "isahc", "surf::"] {
                assert!(
                    !text.contains(http),
                    "{} mentions {http}; only client.rs may talk HTTP",
                    f.display()
                );
            }
        }
    }
    assert!(saw_client);
}
