//! The Web Connector's SOAP 1.1 envelopes, hand-rolled: the schema is eight
//! methods with string parameters and string (or string array, or int) results,
//! fixed since 2005, so a SOAP framework would be more code than this.
//!
//! Requests look like
//!
//! ```xml
//! <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
//!   <soap:Body>
//!     <authenticate xmlns="http://developer.intuit.com/">
//!       <strUserName>…</strUserName><strPassword>…</strPassword>
//!     </authenticate>
//!   </soap:Body>
//! </soap:Envelope>
//! ```
//!
//! and are read by the body's first element name (the `SOAPAction` header says
//! the same thing; the body is what carries the parameters). Responses are the
//! `<methodResponse><methodResult>` shape .NET's `asmx` services produce, which
//! is what the Web Connector was written against.

use quick_xml::escape::escape;

use crate::qbxml::xml::{self, XmlError};

/// The Web Connector's namespace.
pub const NAMESPACE: &str = "http://developer.intuit.com/";

/// One call from the Web Connector. The password is never printed.
#[derive(Clone, PartialEq, Eq)]
pub enum Call {
    ServerVersion,
    ClientVersion {
        version: String,
    },
    Authenticate {
        user_name: String,
        password: String,
    },
    SendRequestXml {
        ticket: String,
        /// `HostQueryRs`, `CompanyQueryRs` and `PreferencesQueryRs` on the first
        /// call of a session; empty after.
        hcp_response: String,
        company_file: String,
        country: String,
        qbxml_major: String,
        qbxml_minor: String,
    },
    ReceiveResponseXml {
        ticket: String,
        response: String,
        /// Set (with `message`) when QuickBooks or the Web Connector failed
        /// instead of answering.
        hresult: String,
        message: String,
    },
    GetLastError {
        ticket: String,
    },
    ConnectionError {
        ticket: String,
        hresult: String,
        message: String,
    },
    CloseConnection {
        ticket: String,
    },
}

impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Call::ServerVersion => f.write_str("ServerVersion"),
            Call::ClientVersion { version } => write!(f, "ClientVersion({version})"),
            Call::Authenticate { user_name, .. } => {
                write!(f, "Authenticate({user_name}, [redacted])")
            }
            Call::SendRequestXml {
                ticket,
                company_file,
                qbxml_major,
                qbxml_minor,
                ..
            } => write!(
                f,
                "SendRequestXml({ticket}, {company_file:?}, qbXML {qbxml_major}.{qbxml_minor})"
            ),
            Call::ReceiveResponseXml {
                ticket,
                response,
                hresult,
                message,
            } => write!(
                f,
                "ReceiveResponseXml({ticket}, {} bytes, hresult {hresult:?}, {message:?})",
                response.len()
            ),
            Call::GetLastError { ticket } => write!(f, "GetLastError({ticket})"),
            Call::ConnectionError {
                ticket,
                hresult,
                message,
            } => write!(f, "ConnectionError({ticket}, {hresult}, {message:?})"),
            Call::CloseConnection { ticket } => write!(f, "CloseConnection({ticket})"),
        }
    }
}

impl Call {
    /// The method name, as the Web Connector spells it.
    pub fn method(&self) -> &'static str {
        match self {
            Call::ServerVersion => "serverVersion",
            Call::ClientVersion { .. } => "clientVersion",
            Call::Authenticate { .. } => "authenticate",
            Call::SendRequestXml { .. } => "sendRequestXML",
            Call::ReceiveResponseXml { .. } => "receiveResponseXML",
            Call::GetLastError { .. } => "getLastError",
            Call::ConnectionError { .. } => "connectionError",
            Call::CloseConnection { .. } => "closeConnection",
        }
    }

    /// The session ticket the call carries, if the method has one.
    pub fn ticket(&self) -> Option<&str> {
        match self {
            Call::SendRequestXml { ticket, .. }
            | Call::ReceiveResponseXml { ticket, .. }
            | Call::GetLastError { ticket }
            | Call::ConnectionError { ticket, .. }
            | Call::CloseConnection { ticket } => Some(ticket),
            _ => None,
        }
    }
}

/// Why a request couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SoapError {
    #[error("the SOAP envelope isn't well-formed XML: {0}")]
    Malformed(String),
    #[error("the SOAP envelope has no Body")]
    NoBody,
    #[error("the SOAP body is empty")]
    EmptyBody,
    #[error("{0:?} isn't a Web Connector method")]
    UnknownMethod(String),
}

impl From<XmlError> for SoapError {
    fn from(e: XmlError) -> Self {
        SoapError::Malformed(e.to_string())
    }
}

/// Read one Web Connector call out of a SOAP request body.
pub fn parse_call(body: &str) -> Result<Call, SoapError> {
    let root = xml::parse_tree(body)?;
    let soap_body = if root.local_name() == "Envelope" {
        root.child("Body").ok_or(SoapError::NoBody)?
    } else if root.local_name() == "Body" {
        &root
    } else {
        return Err(SoapError::NoBody);
    };
    let call = soap_body.children.first().ok_or(SoapError::EmptyBody)?;
    let param =
        |name: &str| -> String { call.child(name).map(|n| n.text.clone()).unwrap_or_default() };
    Ok(match call.local_name() {
        "serverVersion" => Call::ServerVersion,
        "clientVersion" => Call::ClientVersion {
            version: param("strVersion"),
        },
        "authenticate" => Call::Authenticate {
            user_name: param("strUserName"),
            password: param("strPassword"),
        },
        "sendRequestXML" => Call::SendRequestXml {
            ticket: param("ticket"),
            hcp_response: param("strHCPResponse"),
            company_file: param("strCompanyFileName"),
            country: param("qbXMLCountry"),
            qbxml_major: param("qbXMLMajorVers"),
            qbxml_minor: param("qbXMLMinorVers"),
        },
        "receiveResponseXML" => Call::ReceiveResponseXml {
            ticket: param("ticket"),
            response: param("response"),
            hresult: param("hresult"),
            message: param("message"),
        },
        "getLastError" => Call::GetLastError {
            ticket: param("ticket"),
        },
        "connectionError" => Call::ConnectionError {
            ticket: param("ticket"),
            hresult: param("hresult"),
            message: param("message"),
        },
        "closeConnection" => Call::CloseConnection {
            ticket: param("ticket"),
        },
        other => return Err(SoapError::UnknownMethod(other.to_owned())),
    })
}

/// What `authenticate` tells the Web Connector in its second string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthOutcome {
    /// Wrong user name or password ("not valid user").
    Invalid,
    /// Signed in, nothing to do right now.
    NothingToDo,
    /// Signed in, work queued: use the company QuickBooks has open (empty
    /// string) or open this company file.
    Work { company_file: String },
    /// Signed in, but another session is running.
    Busy,
}

fn envelope(inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\
         <soap:Body>{inner}</soap:Body></soap:Envelope>"
    )
}

/// A `<methodResponse><methodResult>value</methodResult></methodResponse>`.
pub fn string_response(method: &str, value: &str) -> String {
    envelope(&format!(
        "<{method}Response xmlns=\"{NAMESPACE}\"><{method}Result>{}</{method}Result></{method}Response>",
        escape(value)
    ))
}

/// `receiveResponseXML`'s integer result.
pub fn int_response(method: &str, value: i32) -> String {
    envelope(&format!(
        "<{method}Response xmlns=\"{NAMESPACE}\"><{method}Result>{value}</{method}Result></{method}Response>"
    ))
}

/// `authenticate`'s string array: the ticket, then the outcome.
pub fn authenticate_response(ticket: &str, outcome: &AuthOutcome) -> String {
    let second = match outcome {
        AuthOutcome::Invalid => "nvu",
        AuthOutcome::NothingToDo => "none",
        AuthOutcome::Busy => "busy",
        AuthOutcome::Work { company_file } => company_file.as_str(),
    };
    envelope(&format!(
        "<authenticateResponse xmlns=\"{NAMESPACE}\"><authenticateResult>\
         <string>{}</string><string>{}</string>\
         </authenticateResult></authenticateResponse>",
        escape(ticket),
        escape(second)
    ))
}

/// A SOAP fault, for a body that couldn't be read.
pub fn fault(message: &str) -> String {
    envelope(&format!(
        "<soap:Fault><faultcode>soap:Client</faultcode><faultstring>{}</faultstring></soap:Fault>",
        escape(message)
    ))
}

/// The `Content-Type` of every SOAP message.
pub const CONTENT_TYPE: &str = "text/xml; charset=utf-8";

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn reads_the_web_connectors_calls() {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
  <soap:Body>
    <authenticate xmlns="http://developer.intuit.com/">
      <strUserName>books</strUserName>
      <strPassword>s3cret&amp;</strPassword>
    </authenticate>
  </soap:Body>
</soap:Envelope>"#;
        let call = parse_call(body).unwrap();
        assert_eq!(
            call,
            Call::Authenticate {
                user_name: "books".into(),
                password: "s3cret&".into()
            }
        );
        assert_eq!(format!("{call:?}"), "Authenticate(books, [redacted])");

        let body = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body>
<sendRequestXML xmlns="http://developer.intuit.com/"><ticket>t1</ticket><strHCPResponse>&lt;QBXML/&gt;</strHCPResponse>
<strCompanyFileName>C:\Books\Co.QBW</strCompanyFileName><qbXMLCountry>US</qbXMLCountry><qbXMLMajorVers>13</qbXMLMajorVers><qbXMLMinorVers>0</qbXMLMinorVers></sendRequestXML>
</soap:Body></soap:Envelope>"#;
        let call = parse_call(body).unwrap();
        assert_eq!(call.ticket(), Some("t1"));
        assert_eq!(call.method(), "sendRequestXML");
        if let Call::SendRequestXml {
            hcp_response,
            company_file,
            qbxml_major,
            ..
        } = call
        {
            assert_eq!(hcp_response, "<QBXML/>");
            assert_eq!(company_file, r"C:\Books\Co.QBW");
            assert_eq!(qbxml_major, "13");
        }
        let body = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body>
<receiveResponseXML xmlns="http://developer.intuit.com/"><ticket>t1</ticket><response></response><hresult>0x80040400</hresult><message>parse error</message></receiveResponseXML>
</soap:Body></soap:Envelope>"#;
        assert_eq!(
            parse_call(body).unwrap(),
            Call::ReceiveResponseXml {
                ticket: "t1".into(),
                response: String::new(),
                hresult: "0x80040400".into(),
                message: "parse error".into()
            }
        );
        for (m, want) in [
            ("serverVersion", Call::ServerVersion),
            (
                "getLastError",
                Call::GetLastError {
                    ticket: String::new(),
                },
            ),
            (
                "closeConnection",
                Call::CloseConnection {
                    ticket: String::new(),
                },
            ),
        ] {
            let body = format!(
                "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><{m} xmlns=\"http://developer.intuit.com/\"/></soap:Body></soap:Envelope>"
            );
            assert_eq!(parse_call(&body).unwrap(), want);
        }
        assert_eq!(
            parse_call(
                "<soap:Envelope xmlns:soap=\"x\"><soap:Body><frobnicate/></soap:Body></soap:Envelope>"
            ),
            Err(SoapError::UnknownMethod("frobnicate".into()))
        );
        assert_eq!(parse_call("<Envelope/>"), Err(SoapError::NoBody));
        assert!(matches!(
            parse_call("<Envelope><Body>"),
            Err(SoapError::Malformed(_))
        ));
    }

    #[test]
    fn writes_asmx_shaped_responses() {
        let r = authenticate_response("abc", &AuthOutcome::NothingToDo);
        assert!(r.contains(
            "<authenticateResponse xmlns=\"http://developer.intuit.com/\"><authenticateResult><string>abc</string><string>none</string></authenticateResult></authenticateResponse>"
        ), "{r}");
        let r = authenticate_response(
            "abc",
            &AuthOutcome::Work {
                company_file: String::new(),
            },
        );
        assert!(r.contains("<string>abc</string><string></string>"));
        let r = string_response("sendRequestXML", "<QBXML><a/></QBXML>");
        assert!(r.contains(
            "<sendRequestXMLResult>&lt;QBXML&gt;&lt;a/&gt;&lt;/QBXML&gt;</sendRequestXMLResult>"
        ));
        assert!(
            int_response("receiveResponseXML", -1)
                .contains("<receiveResponseXMLResult>-1</receiveResponseXMLResult>")
        );
        assert!(fault("bad").contains("<faultstring>bad</faultstring>"));
        // Every response parses back as XML.
        for r in [r, fault("x"), int_response("receiveResponseXML", 100)] {
            crate::qbxml::xml::to_json(&r).unwrap();
        }
    }
}
