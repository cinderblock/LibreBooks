//! Which QuickBooks a company syncs from through the SDK, and the guard that
//! keeps a company file away from the wrong one.
//!
//! **Why.** Opening a company file in a newer QuickBooks upgrades it in place,
//! and the older QuickBooks can't open it again. With two installed (say
//! Enterprise 14.0 and Pro 2021), the SDK, given a file path and no QuickBooks
//! running, starts whichever QuickBooks it likes. So LibreBooks never lets it
//! start one: the chosen QuickBooks must already be running, and no other
//! version beside it. After connecting, the session also checks that
//! QuickBooks reports the chosen version (`HostQuery`), before reading anything.

use lb_qbsdk::{Install, Running};
use serde::Serialize;
use ts_rs::TS;

/// A QuickBooks on this machine, for the import screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct QuickBooksInstall {
    /// QuickBooks' major version (24 is 2014 / Enterprise 14.0, 31 is 2021).
    pub major: u32,
    /// What QuickBooks calls itself ("QuickBooks Pro 2021").
    pub product: String,
    pub exe: String,
    /// Running in this Windows session right now.
    pub running: bool,
}

/// The installs, each marked running or not.
pub fn describe(installs: &[Install], running: &[Running]) -> Vec<QuickBooksInstall> {
    installs
        .iter()
        .map(|i| QuickBooksInstall {
            major: i.major,
            product: i.product.clone(),
            exe: i.exe.display().to_string(),
            running: running.iter().any(|r| r.major == Some(i.major)),
        })
        .collect()
}

fn product(installs: &[Install], major: u32) -> String {
    installs
        .iter()
        .find(|i| i.major == major)
        .map_or_else(|| format!("QuickBooks version {major}"), |i| i.product.clone())
}

/// The version a company syncs from: its choice, or the only one installed.
/// `None` when nothing is installed that LibreBooks can see (it can't check a
/// version then, only that some QuickBooks is running).
pub fn chosen(choice: Option<u32>, installs: &[Install]) -> Result<Option<u32>, String> {
    match (choice, installs) {
        (Some(v), []) => Ok(Some(v)),
        (Some(v), _) if installs.iter().any(|i| i.major == v) => Ok(Some(v)),
        (Some(v), _) => Err(format!(
            "this company syncs from QuickBooks version {v}, which isn't installed here (installed: {}). Choose another under Connection.",
            list(installs)
        )),
        (None, []) => Ok(None),
        (None, [only]) => Ok(Some(only.major)),
        (None, _) => Err(format!(
            "more than one QuickBooks is installed here ({}). Choose which one this company syncs from under Connection: opening a company file in a newer QuickBooks upgrades it, and the older one can't open it again.",
            list(installs)
        )),
    }
}

fn list(installs: &[Install]) -> String {
    installs
        .iter()
        .map(|i| i.product.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether the SDK may connect now. `Ok` carries the version QuickBooks must
/// report once connected.
pub fn check(
    choice: Option<u32>,
    installs: &[Install],
    running: &[Running],
) -> Result<Option<u32>, String> {
    let want = chosen(choice, installs)?;
    let wanted = want.map_or_else(|| "QuickBooks".to_owned(), |v| product(installs, v));
    if running.is_empty() {
        return Err(format!(
            "QuickBooks isn't running. Open the company file in {wanted} first, then sync. LibreBooks doesn't start QuickBooks itself, so a company file is never opened by the wrong version."
        ));
    }
    if let Some(v) = want
        && let Some(other) = running.iter().find(|r| r.major != Some(v))
    {
        let which = other.major.map_or_else(
            || format!("another QuickBooks ({})", other.exe.display()),
            |m| product(installs, m),
        );
        return Err(format!(
            "{which} is running, but this company syncs from {wanted}. Close it first: the SDK talks to whichever QuickBooks is running."
        ));
    }
    Ok(want)
}

/// What `open_in_quickbooks` should start, after the same checks: the chosen
/// install, and only when no QuickBooks is running.
pub fn to_open<'a>(
    choice: Option<u32>,
    installs: &'a [Install],
    running: &[Running],
    company_file: &str,
) -> Result<&'a Install, String> {
    let file = company_file.trim();
    if file.is_empty() {
        return Err("set the company file first".into());
    }
    if !std::path::Path::new(file)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("qbw"))
    {
        return Err(format!("{file} isn't a QuickBooks company file (.QBW)"));
    }
    let Some(v) = chosen(choice, installs)? else {
        return Err("no QuickBooks is installed here that LibreBooks can find".into());
    };
    if let Some(r) = running.first() {
        let which = r
            .major
            .map_or_else(|| r.exe.display().to_string(), |m| product(installs, m));
        return Err(format!(
            "{which} is already running. Open the company file from its File menu, or close it and try again."
        ));
    }
    installs
        .iter()
        .find(|i| i.major == v)
        .ok_or_else(|| format!("QuickBooks version {v} isn't installed here"))
}

/// `MajorVersion` from a `HostQueryRs`.
pub fn host_major(xml: &str) -> Option<u32> {
    let start = xml.find("<MajorVersion>")? + "<MajorVersion>".len();
    let end = start + xml[start..].find("</MajorVersion>")?;
    xml[start..end].trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn install(major: u32, product: &str) -> Install {
        Install {
            major,
            product: product.into(),
            exe: PathBuf::from(format!(r"C:\QB{major}\qbw32.exe")),
        }
    }

    fn run(major: Option<u32>) -> Running {
        Running {
            pid: 1,
            major,
            exe: PathBuf::from(r"C:\QB\qbw32.exe"),
        }
    }

    fn two() -> Vec<Install> {
        vec![
            install(24, "QuickBooks Enterprise Solutions: Accountant Edition 14.0"),
            install(31, "QuickBooks Pro 2021"),
        ]
    }

    #[test]
    fn with_two_installed_a_company_must_choose() {
        let e = check(None, &two(), &[run(Some(24))]).unwrap_err();
        assert!(e.contains("more than one QuickBooks"), "{e}");
    }

    #[test]
    fn the_chosen_one_running_alone_is_fine() {
        assert_eq!(check(Some(24), &two(), &[run(Some(24))]), Ok(Some(24)));
    }

    #[test]
    fn another_version_running_refuses() {
        let e = check(Some(24), &two(), &[run(Some(31))]).unwrap_err();
        assert!(e.starts_with("QuickBooks Pro 2021 is running"), "{e}");
        // Both running: still refused, since the SDK may reach either.
        assert!(check(Some(24), &two(), &[run(Some(24)), run(Some(31))]).is_err());
        // A QuickBooks that matches no install can't be vouched for.
        assert!(check(Some(24), &two(), &[run(None)]).is_err());
    }

    #[test]
    fn nothing_running_refuses_rather_than_letting_the_sdk_start_one() {
        let e = check(Some(24), &two(), &[]).unwrap_err();
        assert!(e.contains("Open the company file in QuickBooks Enterprise Solutions: Accountant Edition 14.0"), "{e}");
    }

    #[test]
    fn one_install_needs_no_choice() {
        let one = [install(31, "QuickBooks Pro 2021")];
        assert_eq!(check(None, &one, &[run(Some(31))]), Ok(Some(31)));
    }

    #[test]
    fn a_choice_that_isnt_installed_says_so() {
        let e = check(Some(29), &two(), &[run(Some(31))]).unwrap_err();
        assert!(e.contains("isn't installed"), "{e}");
    }

    #[test]
    fn opening_starts_the_chosen_install_only_when_nothing_runs() {
        let installs = two();
        let i = to_open(Some(24), &installs, &[], r"C:\Old\Shop.QBW").unwrap();
        assert_eq!(i.major, 24);
        assert!(to_open(Some(24), &installs, &[run(Some(24))], r"C:\Old\Shop.QBW").is_err());
        assert!(to_open(Some(24), &installs, &[], "").is_err());
        assert!(to_open(Some(24), &installs, &[], r"C:\Old\Shop.QBB").is_err());
        assert!(to_open(None, &installs, &[], r"C:\Old\Shop.QBW").is_err());
    }

    #[test]
    fn host_query_major_version() {
        let xml = "<QBXML><QBXMLMsgsRs><HostQueryRs statusCode=\"0\"><HostRet><ProductName>QuickBooks Pro 2021</ProductName><MajorVersion>31</MajorVersion><MinorVersion>0</MinorVersion></HostRet></HostQueryRs></QBXMLMsgsRs></QBXML>";
        assert_eq!(host_major(xml), Some(31));
        assert_eq!(host_major("<HostRet/>"), None);
    }
}
