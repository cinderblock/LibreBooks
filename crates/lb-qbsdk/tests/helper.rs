//! The helper as a program. These never send a valid hello: on a machine with
//! QuickBooks that would open the user's company.

use std::process::{Command, Stdio};

use lb_qbsdk::wire;

const HELPER: &str = env!("CARGO_BIN_EXE_librebooks-qbsdk");

#[test]
fn any_argument_prints_what_it_is() {
    let out = Command::new(HELPER).arg("--version").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("librebooks-qbsdk "), "{text}");
    assert!(
        text.contains(&format!("protocol {}", wire::VERSION)),
        "{text}"
    );
}

#[cfg(windows)]
fn answer_to(frame: &[u8]) -> Option<Result<String, String>> {
    use std::io::Write;

    let mut child = Command::new(HELPER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    wire::write_frame(&mut stdin, frame).unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    wire::read_reply(&mut out.stdout.as_slice()).unwrap()
}

#[cfg(windows)]
#[test]
fn a_stale_app_is_told_so() {
    let mut hello = wire::hello(&lb_qbsdk::Config {
        app_id: String::new(),
        app_name: "test".into(),
        company_file: String::new(),
        file_mode: lb_qbsdk::FileMode::DontCare,
    });
    hello[4] = wire::VERSION + 1;
    let e = answer_to(&hello).unwrap().unwrap_err();
    assert!(e.contains("out of step"), "{e}");
}

#[cfg(windows)]
#[test]
fn nonsense_is_refused_before_any_session() {
    let e = answer_to(b"not a hello").unwrap().unwrap_err();
    assert!(e.contains("malformed"), "{e}");
}

#[test]
fn nobody_there_ends_quietly() {
    let out = Command::new(HELPER).stdin(Stdio::null()).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(out.stdout, Vec::<u8>::new());
}
