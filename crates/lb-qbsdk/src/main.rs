//! `librebooks-qbsdk`: holds the QuickBooks SDK session in a 32-bit process for
//! a 64-bit LibreBooks. LibreBooks starts it when it can't reach QuickBooks
//! itself; there's no reason to run it by hand. See [`lb_qbsdk::wire`].

use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args().nth(1).is_some() {
        println!(
            "librebooks-qbsdk {} (protocol {}, {}-bit): LibreBooks' helper for reaching the QuickBooks SDK; LibreBooks starts it when it needs it",
            env!("CARGO_PKG_VERSION"),
            lb_qbsdk::wire::VERSION,
            usize::BITS,
        );
        return ExitCode::SUCCESS;
    }
    run()
}

#[cfg(all(windows, feature = "com"))]
fn run() -> ExitCode {
    use lb_qbsdk::wire;

    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let config = match wire::read_frame(&mut input) {
        Ok(Some(bytes)) => match wire::parse_hello(&bytes) {
            Ok(config) => config,
            Err(e) => {
                let _ = wire::write_reply(&mut output, Err(&e));
                return ExitCode::FAILURE;
            }
        },
        // Nobody there: started by hand, or the app has gone.
        Ok(None) | Err(_) => return ExitCode::FAILURE,
    };
    lb_qbsdk::session::run(&config, |opened| {
        let session = match opened {
            Ok(session) => session,
            Err(e) => {
                let _ = wire::write_reply(&mut output, Err(&e.message));
                return;
            }
        };
        if wire::write_reply(&mut output, Ok(session.company_file())).is_err() {
            return;
        }
        while let Ok(Some(frame)) = wire::read_frame(&mut input) {
            let answer = String::from_utf8(frame)
                .map_err(|_| "the helper got a document that isn't UTF-8".to_owned())
                .and_then(|xml| session.exchange(&xml));
            let reply = match &answer {
                Ok(text) => Ok(text.as_str()),
                Err(e) => Err(e.as_str()),
            };
            if wire::write_reply(&mut output, reply).is_err() {
                return;
            }
        }
    });
    ExitCode::SUCCESS
}

#[cfg(not(all(windows, feature = "com")))]
fn run() -> ExitCode {
    eprintln!("librebooks-qbsdk needs Windows and the QuickBooks SDK");
    ExitCode::FAILURE
}
