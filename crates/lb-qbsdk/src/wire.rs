//! What the app and the 32-bit helper say to each other over the helper's
//! stdin and stdout: length-prefixed frames (a little-endian `u32`, then the
//! bytes).
//!
//! 1. The app sends a *hello*: [`MAGIC`], [`VERSION`], the file mode, then the
//!    application id, name and company file, NUL-separated.
//! 2. The helper opens the session and answers with a *reply*: a tag byte
//!    ([`OK`] or [`ERROR`]) and UTF-8 text, here the company file or the reason.
//! 3. Each qbXML document the app sends gets a reply holding QuickBooks'
//!    response or the reason it failed.
//! 4. The app closes stdin; the helper ends the session and exits. That also
//!    happens if the app dies, so a helper is never left holding the company.
//!
//! Self-update replaces only the main binary, so an older helper can meet a
//! newer app: bump [`VERSION`] on any change here, and the helper refuses.

use std::io::{self, Read, Write};

use crate::{Config, FileMode};

/// The name of the helper, beside the main binary.
pub const HELPER_EXE: &str = "librebooks-qbsdk.exe";

const MAGIC: &[u8; 4] = b"LBQS";
pub const VERSION: u8 = 1;
const OK: u8 = 0;
const ERROR: u8 = 1;
/// More than any qbXML response; a length past this is a broken stream.
const MAX_FRAME: u32 = 1 << 30;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Write one frame and flush it.
pub fn write_frame(w: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let len = u32::try_from(bytes.len())
        .ok()
        .filter(|n| *n <= MAX_FRAME)
        .ok_or_else(|| invalid("frame too large"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

/// Read one frame; `None` when the stream ended between frames.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME {
        return Err(invalid(format!("frame of {len} bytes")));
    }
    let mut bytes = vec![0u8; len as usize];
    r.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

/// The hello for `config`.
pub fn hello(config: &Config) -> Vec<u8> {
    let mode = match config.file_mode {
        FileMode::DontCare => 0,
        FileMode::SingleUser => 1,
        FileMode::MultiUser => 2,
    };
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(mode);
    out.extend_from_slice(config.app_id.as_bytes());
    out.push(0);
    out.extend_from_slice(config.app_name.as_bytes());
    out.push(0);
    out.extend_from_slice(config.company_file.as_bytes());
    out
}

/// The config a hello carries, or why it can't be accepted.
pub fn parse_hello(bytes: &[u8]) -> Result<Config, String> {
    let [m0, m1, m2, m3, version, mode, rest @ ..] = bytes else {
        return Err("the helper got a malformed hello".into());
    };
    if [*m0, *m1, *m2, *m3] != *MAGIC {
        return Err("the helper got a malformed hello".into());
    }
    if *version != VERSION {
        return Err(format!(
            "{HELPER_EXE} is out of step with this LibreBooks (protocol {version} asked, {VERSION} here): reinstall it from the same release"
        ));
    }
    let file_mode = match mode {
        0 => FileMode::DontCare,
        1 => FileMode::SingleUser,
        2 => FileMode::MultiUser,
        other => return Err(format!("the helper got an unknown file mode {other}")),
    };
    let rest = std::str::from_utf8(rest).map_err(|_| "the helper got a malformed hello")?;
    let mut parts = rest.split('\0');
    let (Some(app_id), Some(app_name), Some(company_file), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("the helper got a malformed hello".into());
    };
    Ok(Config {
        app_id: app_id.to_owned(),
        app_name: app_name.to_owned(),
        company_file: company_file.to_owned(),
        file_mode,
    })
}

/// Write a reply.
pub fn write_reply(w: &mut impl Write, reply: Result<&str, &str>) -> io::Result<()> {
    let (tag, text) = match reply {
        Ok(text) => (OK, text),
        Err(text) => (ERROR, text),
    };
    let mut out = Vec::with_capacity(text.len() + 1);
    out.push(tag);
    out.extend_from_slice(text.as_bytes());
    write_frame(w, &out)
}

/// Read a reply; `None` when the helper is gone.
pub fn read_reply(r: &mut impl Read) -> io::Result<Option<Result<String, String>>> {
    let Some(frame) = read_frame(r)? else {
        return Ok(None);
    };
    let Some((tag, text)) = frame.split_first() else {
        return Err(invalid("an empty reply"));
    };
    let text = String::from_utf8(text.to_vec()).map_err(|_| invalid("a reply isn't UTF-8"))?;
    match *tag {
        OK => Ok(Some(Ok(text))),
        ERROR => Ok(Some(Err(text))),
        other => Err(invalid(format!("a reply tagged {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            app_id: String::new(),
            app_name: "LibreBooks".into(),
            company_file: r"C:\Books\Ünï côdé.QBW".into(),
            file_mode: FileMode::MultiUser,
        }
    }

    #[test]
    fn a_hello_round_trips() {
        assert_eq!(parse_hello(&hello(&config())).unwrap(), config());
    }

    #[test]
    fn a_stale_helper_says_so_rather_than_guessing() {
        let mut bytes = hello(&config());
        bytes[4] = VERSION + 1;
        let e = parse_hello(&bytes).unwrap_err();
        assert!(e.contains("out of step"), "{e}");
    }

    #[test]
    fn a_bad_hello_is_refused() {
        assert!(parse_hello(b"").is_err());
        assert!(parse_hello(b"NOPE\x01\x00a\0b\0c").is_err());
        // Too few and too many fields, an unknown mode.
        for bytes in [
            &b"LBQS\x01\x00a\0b"[..],
            &b"LBQS\x01\x00a\0b\0c\0d"[..],
            &b"LBQS\x01\x09a\0b\0c"[..],
        ] {
            assert!(parse_hello(bytes).is_err());
        }
    }

    #[test]
    fn frames_and_replies_round_trip_and_end_cleanly() {
        let mut pipe = Vec::new();
        write_reply(&mut pipe, Ok("<QBXML/>")).unwrap();
        write_reply(&mut pipe, Err("Could not start QuickBooks")).unwrap();
        write_frame(&mut pipe, b"").unwrap();
        let mut r = pipe.as_slice();
        assert_eq!(read_reply(&mut r).unwrap(), Some(Ok("<QBXML/>".into())));
        assert_eq!(
            read_reply(&mut r).unwrap(),
            Some(Err("Could not start QuickBooks".into()))
        );
        assert!(read_reply(&mut r).is_err(), "an empty reply is malformed");
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn a_broken_stream_is_an_error_not_a_huge_allocation() {
        let mut r = &u32::MAX.to_le_bytes()[..];
        assert!(read_frame(&mut r).is_err());
        // A frame cut off part way.
        let mut r = &[5u8, 0, 0, 0, b'x'][..];
        assert!(read_frame(&mut r).is_err());
    }
}
