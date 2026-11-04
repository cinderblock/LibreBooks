//! Which QuickBooks are installed on this machine, and which are running in
//! this Windows session.
//!
//! QuickBooks records each install under
//! `HKLM\SOFTWARE\Intuit\QuickBooks\<major>.0\<edition>` (the 32-bit view,
//! `WOW6432Node`, for 2021 and older), with `Product` ("QuickBooks Pro 2021")
//! and `Path` (its `qbw32.exe`). A running QuickBooks is matched to an install
//! by its program's path, so its version is the install's.
//!
//! Read-only: nothing here starts, stops or changes QuickBooks.
//!
//! `unsafe` here is plain Win32 calls (the registry, a process snapshot, a
//! process's image name), each with buffers it owns and handles it closes.

#![allow(unsafe_code)]

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_SAM_FLAGS,
    RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use crate::{Install, Running};

const ROOT: &str = r"SOFTWARE\Intuit\QuickBooks";

/// The QuickBooks installed on this machine, oldest first.
pub fn installed() -> Vec<Install> {
    let mut out: Vec<Install> = Vec::new();
    for view in [KEY_WOW64_32KEY, KEY_WOW64_64KEY] {
        let Some(root) = open(HKEY_LOCAL_MACHINE, ROOT, view) else {
            continue;
        };
        for version in subkeys(root) {
            let Some(major) = version
                .split('.')
                .next()
                .and_then(|m| m.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(vkey) = open(root, &version, view) else {
                continue;
            };
            for edition in subkeys(vkey) {
                let (Some(product), Some(path)) = (
                    string(vkey, &edition, "Product"),
                    string(vkey, &edition, "Path"),
                ) else {
                    continue;
                };
                let exe = PathBuf::from(path);
                if !exe.is_file() || out.iter().any(|i| same_path(&i.exe, &exe)) {
                    continue;
                }
                out.push(Install {
                    major,
                    product,
                    exe,
                });
            }
            close(vkey);
        }
        close(root);
    }
    out.sort_by(|a, b| a.major.cmp(&b.major).then_with(|| a.product.cmp(&b.product)));
    out
}

/// The QuickBooks programs running in this Windows session (the only ones the
/// SDK here can reach), matched against `installs` for their version.
pub fn running(installs: &[Install]) -> Vec<Running> {
    let mut out = Vec::new();
    let Some(mine) = session_of(unsafe { GetCurrentProcessId() }) else {
        return out;
    };
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return out;
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0),
        ..Default::default()
    };
    let mut more = unsafe { Process32FirstW(snap, &raw mut entry) }.is_ok();
    while more {
        let name = wide_to_string(&entry.szExeFile);
        let pid = entry.th32ProcessID;
        if (name.eq_ignore_ascii_case("qbw32.exe") || name.eq_ignore_ascii_case("qbw.exe"))
            && session_of(pid) == Some(mine)
        {
            let exe = image_path(pid);
            let major = exe.as_ref().and_then(|exe| {
                installs
                    .iter()
                    .find(|i| same_path(&i.exe, exe))
                    .map(|i| i.major)
            });
            out.push(Running {
                pid,
                major,
                exe: exe.unwrap_or_else(|| PathBuf::from(name)),
            });
        }
        more = unsafe { Process32NextW(snap, &raw mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snap) };
    out
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

fn session_of(pid: u32) -> Option<u32> {
    let mut session = 0u32;
    unsafe { ProcessIdToSessionId(pid, &raw mut session) }
        .ok()
        .map(|()| session)
}

fn image_path(pid: u32) -> Option<PathBuf> {
    let handle: HANDLE =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = vec![0u16; 1024];
    let mut len = u32::try_from(buf.len()).unwrap_or(0);
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(handle) };
    ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn open(parent: HKEY, sub: &str, view: REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();
    let sub = HSTRING::from(sub);
    let r = unsafe { RegOpenKeyExW(parent, &sub, None, KEY_READ | view, &raw mut key) };
    (r == ERROR_SUCCESS).then_some(key)
}

fn close(key: HKEY) {
    let _ = unsafe { RegCloseKey(key) };
}

fn subkeys(key: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    for index in 0u32.. {
        let mut buf = [0u16; 256];
        let mut len = u32::try_from(buf.len()).unwrap_or(0);
        let r = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(buf.as_mut_ptr())),
                &raw mut len,
                None,
                None,
                None,
                None,
            )
        };
        if r != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

fn string(key: HKEY, sub: &str, value: &str) -> Option<String> {
    let sub = HSTRING::from(sub);
    let value = HSTRING::from(value);
    let mut bytes = 0u32;
    let r = unsafe {
        RegGetValueW(
            key,
            PCWSTR(sub.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&raw mut bytes),
        )
    };
    if r != ERROR_SUCCESS || bytes < 2 {
        return None;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    let r = unsafe {
        RegGetValueW(
            key,
            PCWSTR(sub.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut bytes),
        )
    };
    (r == ERROR_SUCCESS).then(|| wide_to_string(&buf)).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_never_fails() {
        // Whatever is (or isn't) installed here, the listing returns, and every
        // entry points at a program that exists.
        let installs = installed();
        for i in &installs {
            assert!(i.exe.is_file(), "{}", i.exe.display());
        }
        let _ = running(&installs);
    }
}
