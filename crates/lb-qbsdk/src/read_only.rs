//! The check that keeps LibreBooks from ever writing to QuickBooks.

use quick_xml::Reader;
use quick_xml::events::Event;

/// Refuse any document that isn't made only of `…QueryRq` messages.
pub fn check(xml: &str) -> Result<(), String> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut messages = 0usize;
    let mut seen_root = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("refusing to send a document that doesn't parse: {e}"))?;
        let (name, opens) = match &event {
            Event::Start(e) => (e.name(), true),
            Event::Empty(e) => (e.name(), false),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let name = name.into_inner().to_owned();
        match depth {
            0 => {
                if seen_root {
                    return Err(format!("refusing to send a second root element {name:?}"));
                }
                seen_root = true;
                if name != "QBXML" {
                    return Err(format!(
                        "refusing to send a document whose root is {name:?}, not QBXML"
                    ));
                }
            }
            1 if name != "QBXMLMsgsRq" => {
                return Err(format!(
                    "refusing to send {name:?}: only QBXMLMsgsRq is allowed"
                ));
            }
            2 => {
                messages += 1;
                if !name.ends_with("QueryRq") {
                    return Err(format!(
                        "refusing to send {name:?} to QuickBooks: LibreBooks only ever queries"
                    ));
                }
            }
            _ => {}
        }
        if opens {
            depth += 1;
        }
    }
    if depth != 0 {
        return Err("refusing to send a document that doesn't parse: unclosed element".into());
    }
    if !seen_root {
        return Err("refusing to send a document with no root element".into());
    }
    if messages == 0 {
        return Err("refusing to send an empty qbXML document".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn queries_pass() {
        check(
            "<?xml version=\"1.0\"?><?qbxml version=\"13.0\"?><QBXML><QBXMLMsgsRq onError=\"stopOnError\">\
             <CheckQueryRq requestID=\"1\" iterator=\"Start\"><MaxReturned>5</MaxReturned></CheckQueryRq>\
             <HostQueryRq/></QBXMLMsgsRq></QBXML>",
        )
        .unwrap();
    }

    #[test]
    fn writes_are_refused_however_they_arrive() {
        for forbidden in [
            "CheckAddRq",
            "CustomerModRq",
            "ListDelRq",
            "TxnVoidRq",
            "TxnDelRq",
            "DataExtAddRq",
        ] {
            let doc = format!(
                "<?xml version=\"1.0\"?><QBXML><QBXMLMsgsRq onError=\"stopOnError\"><{forbidden}/></QBXMLMsgsRq></QBXML>"
            );
            let e = check(&doc).unwrap_err();
            assert!(e.contains(forbidden), "{e}");
        }
        // Beside a query, in a second message block, and open-and-closed.
        for doc in [
            "<QBXML><QBXMLMsgsRq><CheckQueryRq/><CheckAddRq/></QBXMLMsgsRq></QBXML>",
            "<QBXML><QBXMLMsgsRq><CheckQueryRq/></QBXMLMsgsRq><QBXMLMsgsRq><CheckAddRq></CheckAddRq></QBXMLMsgsRq></QBXML>",
        ] {
            assert!(check(doc).unwrap_err().contains("only ever queries"));
        }
    }

    #[test]
    fn a_name_nested_inside_a_query_is_its_own_business() {
        // Only the messages themselves are requests; a query's fields aren't.
        check("<QBXML><QBXMLMsgsRq><CheckQueryRq><TxnDelRq/></CheckQueryRq></QBXMLMsgsRq></QBXML>")
            .unwrap();
    }

    #[test]
    fn anything_else_is_refused() {
        for doc in [
            "",
            "not xml",
            "<QBXML><QBXMLMsgsRq/></QBXML>",
            "<QBXML/>",
            "<QBXML><QBXMLMsgsRs><CheckQueryRs/></QBXMLMsgsRs></QBXML>",
            "<Other><QBXMLMsgsRq><CheckQueryRq/></QBXMLMsgsRq></Other>",
            "<QBXML><QBXMLMsgsRq><CheckQueryRq/></QBXMLMsgsRq></QBXML><QBXML/>",
            "<QBXML><QBXMLMsgsRq><CheckQueryRq/>",
        ] {
            assert!(check(doc).is_err(), "{doc:?} should be refused");
        }
    }
}
