//! OFX, QFX and QBO files: the bank downloads QuickBooks and Quicken read.
//!
//! All four flavours are the same format underneath:
//!
//! - **OFX 1.x** is SGML: a block of `KEY:VALUE` header lines, then tags in which
//!   *elements* (leaves with a value) are usually not closed (`<TRNAMT>-12.00`)
//!   while *aggregates* (`<STMTTRN>…</STMTTRN>`) are.
//! - **OFX 2.x** is XML: an `<?xml?>` and an `<?OFX?>` processing instruction,
//!   then the same tags, all closed.
//! - **QFX** (Quicken) and **QBO** (QuickBooks Web Connect) are OFX with Intuit's
//!   `<INTU.BID>` added to the sign-on block.
//!
//! One tolerant reader handles all of them: it tokenises tags and text, treats a
//! tag followed by text as a leaf (closing tag optional), anything else as an
//! aggregate, and when an aggregate's closing tag arrives it closes everything
//! still open inside it. An empty SGML element (`<MEMO>` with no value) that was
//! opened as if it were an aggregate is turned back into an empty leaf at that
//! point, and anything wrongly nested under it is moved back up. The result is a
//! plain tree, from which statements are read by name, so tag order, unknown
//! tags, lower-case tags, missing closing tags and stray whitespace don't matter.

use std::collections::BTreeMap;

use lb_core::Money;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{FeedError, Result};
use crate::statement::{AccountKind, Statement, StatementTxn, TrnType};
use crate::text;

/// A node of an OFX document: an aggregate with children, or a leaf with a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Tag name, upper-cased.
    pub name: String,
    /// `Some` for leaves (possibly empty), `None` for aggregates.
    pub value: Option<String>,
    pub children: Vec<Node>,
}

impl Node {
    fn aggregate(name: &str) -> Node {
        Node {
            name: name.to_owned(),
            value: None,
            children: Vec::new(),
        }
    }

    fn leaf(name: &str, value: &str) -> Node {
        Node {
            name: name.to_owned(),
            value: Some(value.to_owned()),
            children: Vec::new(),
        }
    }

    /// The first direct child with this name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// The trimmed, non-empty value of a direct child leaf.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.child(name)
            .and_then(|c| c.value.as_deref())
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    /// Follow a path of child names.
    pub fn path(&self, names: &[&str]) -> Option<&Node> {
        let mut cur = self;
        for n in names {
            cur = cur.child(n)?;
        }
        Some(cur)
    }

    /// Every descendant with this name (not looking inside the ones found).
    pub fn find_all<'a>(&'a self, name: &str) -> Vec<&'a Node> {
        let mut out = Vec::new();
        self.collect(name, &mut out);
        out
    }

    fn collect<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            } else {
                c.collect(name, out);
            }
        }
    }

    /// The node as JSON: leaves are strings, aggregates objects, repeated names
    /// arrays. This is what is kept as a transaction's `raw`.
    pub fn to_json(&self) -> serde_json::Value {
        if let Some(v) = &self.value {
            return serde_json::Value::String(v.trim().to_owned());
        }
        let mut map = serde_json::Map::new();
        for c in &self.children {
            let v = c.to_json();
            match map.get_mut(&c.name) {
                None => {
                    map.insert(c.name.clone(), v);
                }
                Some(serde_json::Value::Array(a)) => a.push(v),
                Some(existing) => {
                    let first = existing.take();
                    *existing = serde_json::Value::Array(vec![first, v]);
                }
            }
        }
        serde_json::Value::Object(map)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Open(String),
    Close(String),
    Text(String),
}

fn tokenize(s: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with('<') {
            if rest.starts_with("<!--") {
                i += rest.find("-->").map_or(rest.len(), |e| e + 3);
                continue;
            }
            if let Some(body) = rest.strip_prefix("<![CDATA[") {
                let end = body.find("]]>").unwrap_or(body.len());
                out.push(Token::Text(body[..end].to_owned()));
                i += 9 + end + 3.min(body.len() - end);
                continue;
            }
            let Some(end) = rest.find('>') else {
                break; // a truncated file: keep what came before
            };
            let inner = &rest[1..end];
            i += end + 1;
            if inner.starts_with('?') || inner.starts_with('!') {
                continue;
            }
            let (closing, inner) = match inner.strip_prefix('/') {
                Some(r) => (true, r),
                None => (false, inner),
            };
            let self_closing = inner.ends_with('/');
            let name = inner
                .trim_end_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            if name.is_empty() {
                continue;
            }
            if closing {
                out.push(Token::Close(name));
            } else {
                out.push(Token::Open(name.clone()));
                if self_closing {
                    out.push(Token::Close(name));
                }
            }
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            out.push(Token::Text(unescape(&rest[..end])));
            i += end;
        }
    }
    out
}

/// XML's entities and numeric references; anything else (a bare `&` in SGML
/// files, which is common: `AT&T`) is left as written.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let semi = rest.find(';').filter(|&p| p <= 10);
        let decoded = semi.and_then(|p| {
            let entity = &rest[1..p];
            let ch = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((ch, p))
        });
        if let Some((ch, p)) = decoded {
            out.push(ch);
            rest = &rest[p + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Read OFX text (SGML or XML) into a tree under a `#root` node.
pub fn parse_tree(text: &str) -> Node {
    let tokens = tokenize(text);
    let mut stack: Vec<Node> = vec![Node::aggregate("#root")];
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Open(name) => {
                // A tag followed by text is a leaf; its closing tag is optional.
                if let Some(Token::Text(t)) = tokens.get(i + 1)
                    && !t.trim().is_empty()
                {
                    let leaf = Node::leaf(name, t.trim());
                    i += 2;
                    if let Some(Token::Close(c)) = tokens.get(i)
                        && c == name
                    {
                        i += 1;
                    }
                    push_child(&mut stack, leaf);
                    continue;
                }
                // `<MEMO></MEMO>` or `<MEMO/>`: an empty leaf.
                let mut j = i + 1;
                while let Some(Token::Text(t)) = tokens.get(j)
                    && t.trim().is_empty()
                {
                    j += 1;
                }
                if let Some(Token::Close(c)) = tokens.get(j)
                    && c == name
                {
                    push_child(&mut stack, Node::leaf(name, ""));
                    i = j + 1;
                    continue;
                }
                stack.push(Node::aggregate(name));
                i += 1;
            }
            Token::Close(name) => {
                if let Some(pos) = stack.iter().rposition(|n| &n.name == name)
                    && pos > 0
                {
                    while stack.len() > pos + 1 {
                        let unclosed = stack.pop().expect("deeper than pos");
                        let parent = stack.last_mut().expect("root stays");
                        flatten_into(parent, unclosed);
                    }
                    let done = stack.pop().expect("pos > 0");
                    push_child(&mut stack, done);
                }
                // A closing tag with nothing open to close is ignored.
                i += 1;
            }
            Token::Text(_) => i += 1,
        }
    }
    // A truncated file: close whatever is still open.
    while stack.len() > 1 {
        let n = stack.pop().expect("len > 1");
        push_child(&mut stack, n);
    }
    stack.pop().expect("root")
}

fn push_child(stack: &mut [Node], n: Node) {
    stack.last_mut().expect("root stays").children.push(n);
}

/// An element that was opened but never closed was really an empty SGML leaf:
/// make it one, and give what was under it back to its parent.
fn flatten_into(parent: &mut Node, mut unclosed: Node) {
    let kids = std::mem::take(&mut unclosed.children);
    unclosed.value = Some(String::new());
    parent.children.push(unclosed);
    parent.children.extend(kids);
}

/// A status block (`<STATUS><CODE>0<SEVERITY>INFO</STATUS>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OfxStatus {
    pub code: u32,
    pub severity: String,
    pub message: Option<String>,
}

impl OfxStatus {
    fn from_node(n: &Node) -> Option<OfxStatus> {
        let s = n.child("STATUS")?;
        Some(OfxStatus {
            code: s.text("CODE").and_then(|c| c.parse().ok()).unwrap_or(0),
            severity: s.text("SEVERITY").unwrap_or("INFO").to_owned(),
            message: s.text("MESSAGE").map(str::to_owned),
        })
    }

    pub fn is_ok(&self) -> bool {
        // 1 is "client is up to date", a success in practice.
        self.code == 0 || self.code == 1
    }

    /// What the code means, in words, with the bank's own message when it sent
    /// one.
    pub fn describe(&self) -> String {
        let meaning = match self.code {
            0 => "success",
            2000 => "the bank reported a general error",
            2002 => "the bank's server is busy",
            2003 => "the bank doesn't recognise the account",
            2019 => "the bank already processed this request",
            2020 => "the requested dates are not available",
            2028 => "the bank doesn't support this request",
            13504 => "the bank doesn't support this OFX version",
            15000 => "the bank requires a new password; change it on the bank's website",
            15500 => "the bank rejected the user name or password",
            15501 => "the account is in use by another session",
            15502 => "the password is locked; contact the bank",
            15510 => "the bank doesn't recognise this application (client UID)",
            15512 => "the bank requires a one-time authentication step",
            15513 => "the bank sent a one-time code; Direct Connect sign-in needs it",
            _ => "the bank reported an error",
        };
        match &self.message {
            Some(m) => format!("{meaning} (code {}): {m}", self.code),
            None => format!("{meaning} (code {})", self.code),
        }
    }
}

/// Everything read from an OFX document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OfxDocument {
    /// Header fields, `KEY:VALUE` lines for 1.x, `<?OFX?>` attributes for 2.x.
    pub header: BTreeMap<String, String>,
    /// `102`, `220`… when the header says.
    pub version: Option<u32>,
    /// The sign-on response, if the document has one.
    pub signon: Option<OfxStatus>,
    pub statements: Vec<Statement>,
    /// Statement responses that came back with an error status and no statement.
    pub statement_errors: Vec<OfxStatus>,
    /// Things in the file that were skipped.
    pub warnings: Vec<String>,
    /// The whole tree, for callers that want more (account lists, say).
    pub root: Option<Node>,
}

/// Is this probably OFX (rather than CSV)?
pub fn looks_like_ofx(bytes: &[u8]) -> bool {
    let head = text::decode(&bytes[..bytes.len().min(4096)]).to_ascii_uppercase();
    head.contains("OFXHEADER") || head.contains("<OFX>") || head.contains("<OFX ")
}

/// Read an OFX/QFX/QBO document.
pub fn parse(bytes: &[u8]) -> Result<OfxDocument> {
    let text = text::decode(bytes);
    let upper = text.to_ascii_uppercase();
    let Some(ofx_start) = upper.find("<OFX>").or_else(|| upper.find("<OFX ")) else {
        return Err(FeedError::parse(
            "OFX",
            "this isn't an OFX, QFX or QBO file (there is no <OFX> block)",
        ));
    };
    let mut doc = OfxDocument {
        header: read_header(&text[..ofx_start]),
        ..OfxDocument::default()
    };
    doc.version = doc.header.get("VERSION").and_then(|v| v.parse().ok());
    let root = parse_tree(&text[ofx_start..]);
    let ofx = root.child("OFX").unwrap_or(&root);

    let sonrs = ofx.path(&["SIGNONMSGSRSV1", "SONRS"]);
    doc.signon = sonrs.and_then(OfxStatus::from_node);
    let org = sonrs
        .and_then(|s| s.path(&["FI"]))
        .and_then(|fi| fi.text("ORG"));
    let fid = sonrs
        .and_then(|s| s.path(&["FI"]))
        .and_then(|fi| fi.text("FID"));
    let intu_bid = sonrs.and_then(|s| s.text("INTU.BID"));

    for trnrs in ofx
        .find_all("STMTTRNRS")
        .into_iter()
        .chain(ofx.find_all("CCSTMTTRNRS"))
    {
        let has_statement = trnrs.child("STMTRS").is_some() || trnrs.child("CCSTMTRS").is_some();
        if let Some(st) = OfxStatus::from_node(trnrs)
            && !st.is_ok()
            && !has_statement
        {
            doc.statement_errors.push(st);
        }
    }
    for (rs, is_card) in ofx
        .find_all("STMTRS")
        .into_iter()
        .map(|n| (n, false))
        .chain(ofx.find_all("CCSTMTRS").into_iter().map(|n| (n, true)))
    {
        let mut s = read_statement(rs, is_card);
        s.account.institution = org.map(str::to_owned);
        s.account.fid = fid.map(str::to_owned);
        s.account.intu_bid = intu_bid.map(str::to_owned);
        s.finish();
        doc.statements.push(s);
    }
    if !ofx.find_all("INVSTMTRS").is_empty() {
        doc.warnings
            .push("investment statements in this file were skipped".into());
    }
    doc.root = Some(root);
    Ok(doc)
}

/// Read a file's statements, failing (with the bank's reason, when it gave one)
/// if there are none.
pub fn parse_statements(bytes: &[u8]) -> Result<Vec<Statement>> {
    let doc = parse(bytes)?;
    if let Some(s) = &doc.signon
        && !s.is_ok()
    {
        return Err(FeedError::Provider {
            provider: "the bank",
            message: s.describe(),
        });
    }
    if doc.statements.is_empty() {
        if let Some(e) = doc.statement_errors.first() {
            return Err(FeedError::Provider {
                provider: "the bank",
                message: e.describe(),
            });
        }
        return Err(FeedError::parse(
            "OFX",
            "there are no bank or credit card statements in this file",
        ));
    }
    Ok(doc.statements)
}

fn read_header(text: &str) -> BTreeMap<String, String> {
    let mut h = BTreeMap::new();
    // 2.x: <?OFX OFXHEADER="200" VERSION="220" …?>
    let upper = text.to_ascii_uppercase();
    if let Some(start) = upper.find("<?OFX") {
        let body = &text[start + 5..];
        let body = &body[..body.find("?>").unwrap_or(body.len())];
        for part in body.split_whitespace() {
            if let Some((k, v)) = part.split_once('=') {
                h.insert(k.to_ascii_uppercase(), v.trim_matches('"').to_owned());
            }
        }
        return h;
    }
    // 1.x: KEY:VALUE lines.
    for line in text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().trim_start_matches('\u{feff}');
            if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric()) {
                h.insert(k.to_ascii_uppercase(), v.trim().to_owned());
            }
        }
    }
    h
}

fn read_statement(rs: &Node, is_card: bool) -> Statement {
    let mut s = Statement::default();
    let from = if is_card {
        rs.child("CCACCTFROM")
    } else {
        rs.child("BANKACCTFROM")
    };
    let bank_id = from.and_then(|f| f.text("BANKID"));
    if let Some(acct) = from.and_then(|f| f.text("ACCTID")) {
        s.account.set_number(bank_id, acct);
    }
    s.account.bank_id = bank_id.map(str::to_owned);
    s.account.account_type = if is_card {
        AccountKind::CreditCard
    } else {
        from.and_then(|f| f.text("ACCTTYPE"))
            .map_or(AccountKind::Checking, AccountKind::from_ofx)
    };
    s.account.currency = rs.text("CURDEF").map(str::to_owned);

    let list = rs.child("BANKTRANLIST");
    s.start = list
        .and_then(|l| l.text("DTSTART"))
        .and_then(text::parse_ofx_date);
    s.end = list
        .and_then(|l| l.text("DTEND"))
        .and_then(text::parse_ofx_date);
    if let Some(bal) = rs.child("LEDGERBAL") {
        s.ledger_balance = bal.text("BALAMT").and_then(|a| text::parse_amount(a).ok());
        s.balance_date = bal.text("DTASOF").and_then(text::parse_ofx_date);
    }
    s.available_balance = rs
        .child("AVAILBAL")
        .and_then(|b| b.text("BALAMT"))
        .and_then(|a| text::parse_amount(a).ok());

    for (n, t) in rs.find_all("STMTTRN").into_iter().enumerate() {
        match read_txn(t) {
            Ok(txn) => {
                if let Some(action) = t.text("CORRECTACTION") {
                    s.warnings.push(format!(
                        "transaction {} asks to {} an earlier one ({}); review it by hand",
                        txn.fitid,
                        action.to_ascii_lowercase(),
                        t.text("CORRECTFITID").unwrap_or("unknown id")
                    ));
                }
                s.transactions.push(txn);
            }
            Err(why) => s
                .warnings
                .push(format!("transaction {} was skipped: {why}", n + 1)),
        }
    }
    s
}

fn read_txn(t: &Node) -> std::result::Result<StatementTxn, String> {
    let posted = t
        .text("DTPOSTED")
        .or_else(|| t.text("DTUSER"))
        .ok_or("it has no date")?;
    let posted = text::parse_ofx_date(posted).ok_or_else(|| format!("{posted:?} is not a date"))?;
    let amount_text = t.text("TRNAMT").ok_or("it has no amount")?;
    let amount: Money = text::parse_amount(amount_text).map_err(|e| e.to_string())?;
    let mut txn = StatementTxn::new(posted, amount);
    txn.fitid = t
        .text("FITID")
        .or_else(|| t.text("SRVRTID"))
        .unwrap_or_default()
        .to_owned();
    txn.trntype = t
        .text("TRNTYPE")
        .map_or(TrnType::by_sign(amount), TrnType::from_ofx);
    let name = t
        .text("NAME")
        .or_else(|| t.path(&["PAYEE"]).and_then(|p| p.text("NAME")))
        .or_else(|| t.text("EXTDNAME"));
    let memo = t.text("MEMO");
    (txn.payee, txn.memo) = match (name, memo) {
        (Some(n), m) => (Some(n.to_owned()), m.map(str::to_owned)),
        // Some banks put everything in MEMO.
        (None, Some(m)) => (Some(m.to_owned()), None),
        (None, None) => (None, None),
    };
    txn.check_number = t
        .text("CHECKNUM")
        .or_else(|| {
            t.text("REFNUM")
                .filter(|r| txn.trntype == TrnType::Check && r.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|c| c.trim_start_matches('0').to_owned())
        .filter(|c| !c.is_empty());
    txn.raw = t.to_json();
    Ok(txn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgml_leaves_need_no_closing_tags() {
        let root = parse_tree("<OFX><A><B>one<C>two</C><D></A><E>three</OFX>");
        let ofx = root.child("OFX").unwrap();
        let a = ofx.child("A").unwrap();
        assert_eq!(a.text("B"), Some("one"));
        assert_eq!(a.text("C"), Some("two"));
        assert_eq!(a.child("D").unwrap().value.as_deref(), Some(""));
        assert_eq!(ofx.text("E"), Some("three"));
    }

    #[test]
    fn an_empty_sgml_element_gives_back_what_it_swallowed() {
        // <MEMO> has no value, so it looks like an aggregate until </STMTTRN>.
        let root = parse_tree(
            "<STMTTRN><TRNTYPE>DEBIT<MEMO><NAME>Cafe<TRNAMT>-4.50</STMTTRN><STMTTRN><NAME>B</STMTTRN>",
        );
        let all = root.find_all("STMTTRN");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].text("NAME"), Some("Cafe"));
        assert_eq!(all[0].text("TRNAMT"), Some("-4.50"));
        assert_eq!(all[0].text("MEMO"), None);
        assert_eq!(all[1].text("NAME"), Some("B"));
    }

    #[test]
    fn xml_entities_comments_and_cdata() {
        let root = parse_tree(
            "<?xml version=\"1.0\"?><!-- hi --><OFX><NAME>AT&amp;T &#38; Co</NAME><MEMO><![CDATA[a<b]]></MEMO><X/><Y>AT&T</Y></OFX>",
        );
        let ofx = root.child("OFX").unwrap();
        assert_eq!(ofx.text("NAME"), Some("AT&T & Co"));
        assert_eq!(ofx.text("MEMO"), Some("a<b"));
        assert_eq!(ofx.child("X").unwrap().value.as_deref(), Some(""));
        assert_eq!(ofx.text("Y"), Some("AT&T"));
    }

    #[test]
    fn stray_and_truncated_tags_are_survived() {
        let root = parse_tree("<OFX></NOPE><A><B>1</A><C><D>2");
        let ofx = root.child("OFX").unwrap();
        assert_eq!(ofx.path(&["A"]).unwrap().text("B"), Some("1"));
        assert_eq!(ofx.path(&["C"]).unwrap().text("D"), Some("2"));
    }

    #[test]
    fn statuses_say_what_went_wrong() {
        let s = OfxStatus {
            code: 15500,
            severity: "ERROR".into(),
            message: Some("Invalid login".into()),
        };
        assert!(!s.is_ok());
        assert_eq!(
            s.describe(),
            "the bank rejected the user name or password (code 15500): Invalid login"
        );
    }

    #[test]
    fn raw_json_keeps_repeats() {
        let root = parse_tree("<T><A>1<A>2<B><C>3</B></T>");
        let json = root.child("T").unwrap().to_json();
        assert_eq!(json, serde_json::json!({"A": ["1", "2"], "B": {"C": "3"}}));
    }
}
