//! CSV downloads: every bank has one, no two agree.
//!
//! A [`CsvMapping`] says how to read one bank's layout: which column holds the
//! date (and in which order day, month and year are written), whether the
//! amount is one signed column (and which sign means money in) or a debit and a
//! credit column or an amount plus a "debit/credit" type column, and where the
//! payee, memo, check number and the bank's own id are. A mapping names columns
//! by their header text when the file has a header row, so it survives a bank
//! adding or reordering columns, and by position when it doesn't.
//!
//! [`detect`] reads a file and proposes a mapping: it finds the delimiter and the
//! header row (skipping the summary block some banks put on top), recognises the
//! layouts of the common US banks and card issuers, falls back to guessing from
//! column names, and works out the date order from the dates themselves. The
//! person confirms (or corrects) the proposal, and the connection saves it.

use std::collections::BTreeMap;

use jiff::civil::Date;
use lb_core::Money;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{FeedError, Result};
use crate::statement::{Statement, StatementTxn, TrnType};
use crate::text::{self, DateOrder};

/// A column, by header text (case and surrounding spaces ignored) or by
/// zero-based position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, rename = "CsvColumn")]
pub enum Column {
    Name(String),
    Index(u32),
}

impl Column {
    pub fn name(s: &str) -> Column {
        Column::Name(s.to_owned())
    }

    fn resolve(&self, headers: Option<&[String]>) -> Option<usize> {
        match self {
            Column::Index(i) => Some(*i as usize),
            Column::Name(n) => {
                let want = norm(n);
                headers?.iter().position(|h| norm(h) == want)
            }
        }
    }
}

pub use crate::statement::Direction;

/// Where the amount is and which way it points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AmountColumns {
    /// One signed column. `positive_is` says what a positive number means: most
    /// banks write deposits positive (`In`); Amex, Apple Card and Discover write
    /// purchases positive (`Out`).
    Signed {
        column: Column,
        positive_is: Direction,
    },
    /// Separate columns for money out and money in (either may be blank). Signs
    /// in them are ignored.
    DebitCredit { debit: Column, credit: Column },
    /// An unsigned amount and a column saying which way it went; a row is money
    /// out when the type cell is one of `out_values` (case-insensitive).
    AmountAndType {
        amount: Column,
        type_column: Column,
        out_values: Vec<String>,
    },
}

/// How to read one bank's CSV layout. Saved with the connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CsvMapping {
    /// `,`, `;`, tab or `|`.
    pub delimiter: char,
    /// Non-empty rows to skip before the header (or before the first transaction
    /// when there is no header): the summary block some banks put on top.
    pub skip_rows: u32,
    pub has_header: bool,
    pub date: Column,
    pub date_order: DateOrder,
    pub amount: AmountColumns,
    pub payee: Option<Column>,
    pub memo: Option<Column>,
    pub check_number: Option<Column>,
    /// The bank's own id for each transaction, when the file has one (Amex's
    /// "Reference", Bank of America's "Reference Number").
    pub fitid: Option<Column>,
    /// A running balance column, read for the statement's ending balance.
    pub balance: Option<Column>,
}

impl Default for CsvMapping {
    fn default() -> Self {
        CsvMapping {
            delimiter: ',',
            skip_rows: 0,
            has_header: true,
            date: Column::name("Date"),
            date_order: DateOrder::Mdy,
            amount: AmountColumns::Signed {
                column: Column::name("Amount"),
                positive_is: Direction::In,
            },
            payee: Some(Column::name("Description")),
            memo: None,
            check_number: None,
            fitid: None,
            balance: None,
        }
    }
}

/// Layouts [`detect`] recognises by their header rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum CsvLayout {
    ChaseChecking,
    ChaseCard,
    BankOfAmericaChecking,
    BankOfAmericaCard,
    Amex,
    AppleCard,
    CapitalOneCard,
    CapitalOne360,
    WellsFargo,
    Discover,
    /// Columns found by their names; the sign was guessed.
    Generic,
}

impl CsvLayout {
    pub const fn label(self) -> &'static str {
        match self {
            CsvLayout::ChaseChecking => "Chase (checking/savings)",
            CsvLayout::ChaseCard => "Chase (credit card)",
            CsvLayout::BankOfAmericaChecking => "Bank of America (checking/savings)",
            CsvLayout::BankOfAmericaCard => "Bank of America (credit card)",
            CsvLayout::Amex => "American Express",
            CsvLayout::AppleCard => "Apple Card",
            CsvLayout::CapitalOneCard => "Capital One (credit card)",
            CsvLayout::CapitalOne360 => "Capital One 360",
            CsvLayout::WellsFargo => "Wells Fargo",
            CsvLayout::Discover => "Discover",
            CsvLayout::Generic => "Generic",
        }
    }
}

/// What [`detect`] proposes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CsvDetection {
    pub layout: CsvLayout,
    pub mapping: CsvMapping,
    /// True when a known layout matched exactly; false when columns or the sign
    /// were guessed and the person should look before importing.
    pub confident: bool,
    /// The header row as found (empty without one).
    pub headers: Vec<String>,
    /// What was guessed, for the confirmation screen.
    pub notes: Vec<String>,
    /// The file read with the proposed mapping, for the person to check.
    pub preview: Statement,
}

fn norm(s: &str) -> String {
    s.trim()
        .trim_start_matches('\u{feff}')
        .trim_matches('"')
        .trim()
        .to_lowercase()
}

/// Rows of a CSV file, all cells as text (ragged rows allowed; blank rows
/// dropped).
fn read_rows(text: &str, delimiter: char) -> Result<Vec<Vec<String>>> {
    let delim = u8::try_from(delimiter)
        .map_err(|_| FeedError::Invalid(format!("{delimiter:?} can't be a CSV delimiter")))?;
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| FeedError::parse("CSV", e.to_string()))?;
        // Line breaks inside quoted cells come in whatever the file used.
        let row: Vec<String> = rec.iter().map(|c| c.replace("\r\n", "\n")).collect();
        if row.iter().all(String::is_empty) {
            continue;
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Read a CSV file with a mapping.
pub fn parse(bytes: &[u8], mapping: &CsvMapping) -> Result<Statement> {
    let text = text::decode(bytes);
    let rows = read_rows(&text, mapping.delimiter)?;
    let mut rows = rows.into_iter().skip(mapping.skip_rows as usize);
    let headers: Option<Vec<String>> = if mapping.has_header {
        Some(
            rows.next()
                .ok_or_else(|| FeedError::parse("CSV", "the file has no header row"))?,
        )
    } else {
        None
    };
    let h = headers.as_deref();
    let col = |c: &Column, what: &str| -> Result<usize> {
        c.resolve(h).ok_or_else(|| {
            FeedError::parse(
                "CSV",
                format!("the {what} column ({}) isn't in this file", describe(c)),
            )
        })
    };
    let opt = |c: &Option<Column>, what: &str| -> Result<Option<usize>> {
        c.as_ref().map(|c| col(c, what)).transpose()
    };
    let date_col = col(&mapping.date, "date")?;
    let amount = match &mapping.amount {
        AmountColumns::Signed {
            column,
            positive_is,
        } => Amount::Signed(col(column, "amount")?, *positive_is),
        AmountColumns::DebitCredit { debit, credit } => {
            Amount::DebitCredit(col(debit, "debit")?, col(credit, "credit")?)
        }
        AmountColumns::AmountAndType {
            amount,
            type_column,
            out_values,
        } => Amount::Typed(
            col(amount, "amount")?,
            col(type_column, "type")?,
            out_values.iter().map(|v| norm(v)).collect(),
        ),
    };
    let payee_col = opt(&mapping.payee, "payee")?;
    let memo_col = opt(&mapping.memo, "memo")?;
    let check_col = opt(&mapping.check_number, "check number")?;
    let fitid_col = opt(&mapping.fitid, "id")?;
    let balance_col = opt(&mapping.balance, "balance")?;

    let mut s = Statement::default();
    let mut balances: Vec<(usize, Date, Money)> = Vec::new();
    let cell = |row: &[String], i: Option<usize>| -> Option<String> {
        i.and_then(|i| row.get(i))
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty())
    };
    for (n, row) in rows.enumerate() {
        let line = n + 1;
        let Some(date_text) = cell(&row, Some(date_col)) else {
            continue; // blank or footer rows
        };
        let Some(posted) = text::parse_date(&date_text, mapping.date_order) else {
            // Footers ("Total", "End of statement") aren't worth a warning.
            if date_text.chars().any(|c| c.is_ascii_digit()) {
                s.warnings.push(format!(
                    "row {line} was skipped: {date_text:?} is not a date"
                ));
            }
            continue;
        };
        let amount = match read_amount(&row, &amount) {
            Ok(Some(a)) => a,
            // Rows with a date and no amount are balance lines ("Beginning
            // balance as of …"), not transactions.
            Ok(None) => continue,
            Err(e) => {
                s.warnings
                    .push(format!("row {line} ({posted}) was skipped: {e}"));
                continue;
            }
        };
        let mut t = StatementTxn::new(posted, amount);
        t.payee = cell(&row, payee_col);
        t.memo = cell(&row, memo_col).filter(|m| Some(m) != t.payee.as_ref());
        t.check_number = cell(&row, check_col)
            .map(|c| c.trim_start_matches('0').to_owned())
            .filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()));
        if t.check_number.is_some() {
            t.trntype = TrnType::Check;
        }
        t.fitid = cell(&row, fitid_col)
            .map(|f| f.trim_matches('\'').to_owned())
            .unwrap_or_default();
        if let Some(b) = cell(&row, balance_col).and_then(|b| text::parse_amount(&b).ok()) {
            balances.push((n, posted, b));
        }
        t.raw = raw_row(&row, h);
        s.transactions.push(t);
    }
    // The ending balance is the running balance on the latest-dated row; among
    // rows on that date, the last one in an oldest-first file and the first one
    // in a newest-first file.
    if let (Some(first), Some(last), Some(latest)) = (
        balances.first(),
        balances.last(),
        balances.iter().map(|b| b.1).max(),
    ) {
        let newest_first = first.1 > last.1;
        let mut on_latest = balances.iter().filter(|b| b.1 == latest);
        let pick = if newest_first {
            on_latest.next()
        } else {
            on_latest.next_back()
        };
        if let Some((_, d, b)) = pick {
            s.ledger_balance = Some(*b);
            s.balance_date = Some(*d);
        }
    }
    s.finish();
    Ok(s)
}

enum Amount {
    Signed(usize, Direction),
    DebitCredit(usize, usize),
    Typed(usize, usize, Vec<String>),
}

fn read_amount(row: &[String], how: &Amount) -> std::result::Result<Option<Money>, String> {
    let cell = |idx: usize| row.get(idx).map(|c| c.trim()).filter(|c| !c.is_empty());
    let parse = |value: &str| text::parse_amount(value).map_err(|e| e.to_string());
    Ok(match how {
        Amount::Signed(idx, dir) => match cell(*idx) {
            None => None,
            Some(text) => {
                let money = parse(text)?;
                Some(match dir {
                    Direction::In => money,
                    Direction::Out => -money,
                })
            }
        },
        Amount::DebitCredit(debit_idx, credit_idx) => {
            let debit = cell(*debit_idx).map(parse).transpose()?.map(Money::abs);
            let credit = cell(*credit_idx).map(parse).transpose()?.map(Money::abs);
            match (debit, credit) {
                (None, None) => None,
                (out, inn) => Some(inn.unwrap_or_default() - out.unwrap_or_default()),
            }
        }
        Amount::Typed(idx, type_idx, outs) => match cell(*idx) {
            None => None,
            Some(text) => {
                let money = parse(text)?.abs();
                let kind = cell(*type_idx).map(norm).unwrap_or_default();
                Some(if outs.contains(&kind) { -money } else { money })
            }
        },
    })
}

fn raw_row(row: &[String], headers: Option<&[String]>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (i, v) in row.iter().enumerate() {
        let key = headers
            .and_then(|h| h.get(i))
            .map(|h| h.trim().trim_start_matches('\u{feff}').to_owned())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| format!("column {}", i + 1));
        map.insert(key, serde_json::Value::String(v.clone()));
    }
    serde_json::Value::Object(map)
}

fn describe(c: &Column) -> String {
    match c {
        Column::Name(n) => format!("\"{n}\""),
        Column::Index(i) => format!("column {}", i + 1),
    }
}

/// Guess the delimiter from the first lines: the candidate that appears the same
/// (non-zero) number of times on the most lines.
fn detect_delimiter(text: &str) -> char {
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(30)
        .collect();
    let mut best = (',', 0usize);
    for d in [',', ';', '\t', '|'] {
        let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
        for l in &lines {
            let n = count_outside_quotes(l, d);
            if n > 0 {
                *counts.entry(n).or_default() += 1;
            }
        }
        let score = counts.values().copied().max().unwrap_or(0);
        if score > best.1 {
            best = (d, score);
        }
    }
    best.0
}

fn count_outside_quotes(line: &str, d: char) -> usize {
    let mut quoted = false;
    let mut n = 0;
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if c == d && !quoted {
            n += 1;
        }
    }
    n
}

const HEADER_WORDS: &[&str] = &[
    "date",
    "description",
    "amount",
    "debit",
    "credit",
    "payee",
    "memo",
    "balance",
    "check",
    "type",
    "category",
    "merchant",
    "reference",
    "posted",
    "transaction",
    "details",
    "withdrawal",
    "deposit",
    "name",
    "card",
];

fn header_score(row: &[String]) -> usize {
    row.iter()
        .filter(|c| {
            let c = norm(c);
            !c.is_empty()
                && c.len() < 40
                && !c.chars().any(|ch| ch.is_ascii_digit())
                && HEADER_WORDS.iter().any(|w| c.contains(w))
        })
        .count()
}

fn row_has_date(row: &[String]) -> bool {
    row.iter().any(|c| {
        let c = c.trim();
        c.len() <= 30
            && c.chars().any(|ch| ch.is_ascii_digit())
            && (text::parse_date(c, DateOrder::Mdy).is_some()
                || text::parse_date(c, DateOrder::Dmy).is_some())
    })
}

/// Read a CSV file and propose how to import it.
pub fn detect(bytes: &[u8]) -> Result<CsvDetection> {
    let text = text::decode(bytes);
    let delimiter = detect_delimiter(&text);
    let rows = read_rows(&text, delimiter)?;
    if rows.is_empty() {
        return Err(FeedError::parse("CSV", "the file is empty"));
    }
    let mut notes = Vec::new();

    // The header: the first row that looks like column names and is followed by
    // a row with a date in it.
    let header_at = (0..rows.len().min(40))
        .find(|&i| header_score(&rows[i]) >= 2 && rows.get(i + 1).is_some_and(|r| row_has_date(r)));
    let (skip_rows, headers): (u32, Vec<String>) = if let Some(i) = header_at {
        (i as u32, rows[i].clone())
    } else {
        let first_data = (0..rows.len().min(40))
            .find(|&i| row_has_date(&rows[i]))
            .ok_or_else(|| FeedError::parse("CSV", "no row in the first 40 has a date in it"))?;
        (first_data as u32, Vec::new())
    };
    if skip_rows > 0 {
        notes.push(format!(
            "skipping {skip_rows} row(s) above the {}",
            if headers.is_empty() {
                "first transaction"
            } else {
                "header"
            }
        ));
    }
    let data: Vec<&Vec<String>> = rows
        .iter()
        .skip(skip_rows as usize + usize::from(!headers.is_empty()))
        .collect();
    let lower: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    let has = |name: &str| lower.iter().any(|h| h == name);

    let (layout, mut mapping, confident) = if headers.is_empty() {
        known_headerless(&data).map_or_else(
            || {
                (
                    CsvLayout::Generic,
                    generic_headerless(&data, &mut notes),
                    false,
                )
            },
            |m| (CsvLayout::WellsFargo, m, true),
        )
    } else if let Some((layout, m)) = known_layout(&has) {
        (layout, m, true)
    } else {
        let m = generic(&headers, &data, &mut notes)?;
        (CsvLayout::Generic, m, false)
    };
    mapping.delimiter = delimiter;
    mapping.skip_rows = skip_rows;
    mapping.has_header = !headers.is_empty();

    // Date order from the dates themselves.
    let date_idx = mapping.date.resolve(Some(headers.as_slice())).unwrap_or(0);
    let (order, why) = detect_date_order(data.iter().filter_map(|r| r.get(date_idx)));
    mapping.date_order = order;
    if let Some(why) = why {
        notes.push(why);
    }

    let preview = parse(bytes, &mapping)?;
    if layout != CsvLayout::Generic {
        notes.insert(0, format!("recognised a {} export", layout.label()));
    }
    Ok(CsvDetection {
        layout,
        mapping,
        confident,
        headers,
        notes,
        preview,
    })
}

fn known_layout(has: &dyn Fn(&str) -> bool) -> Option<(CsvLayout, CsvMapping)> {
    let signed = |c: &str, dir| AmountColumns::Signed {
        column: Column::name(c),
        positive_is: dir,
    };
    let base = CsvMapping::default();
    if has("details") && has("posting date") && has("description") && has("amount") {
        return Some((
            CsvLayout::ChaseChecking,
            CsvMapping {
                date: Column::name("Posting Date"),
                amount: signed("Amount", Direction::In),
                payee: Some(Column::name("Description")),
                check_number: has("check or slip #").then(|| Column::name("Check or Slip #")),
                balance: has("balance").then(|| Column::name("Balance")),
                ..base
            },
        ));
    }
    if has("transaction date") && has("post date") && has("description") && has("amount") {
        return Some((
            CsvLayout::ChaseCard,
            CsvMapping {
                date: Column::name("Post Date"),
                amount: signed("Amount", Direction::In),
                payee: Some(Column::name("Description")),
                memo: has("memo").then(|| Column::name("Memo")),
                ..base
            },
        ));
    }
    if has("posted date") && has("reference number") && has("payee") && has("amount") {
        return Some((
            CsvLayout::BankOfAmericaCard,
            CsvMapping {
                date: Column::name("Posted Date"),
                amount: signed("Amount", Direction::In),
                payee: Some(Column::name("Payee")),
                memo: has("address").then(|| Column::name("Address")),
                fitid: Some(Column::name("Reference Number")),
                ..base
            },
        ));
    }
    if has("date") && has("description") && has("amount") && has("running bal.") {
        return Some((
            CsvLayout::BankOfAmericaChecking,
            CsvMapping {
                date: Column::name("Date"),
                amount: signed("Amount", Direction::In),
                payee: Some(Column::name("Description")),
                balance: Some(Column::name("Running Bal.")),
                ..base
            },
        ));
    }
    if has("transaction date") && has("clearing date") && has("merchant") && has("amount (usd)") {
        return Some((
            CsvLayout::AppleCard,
            CsvMapping {
                date: Column::name("Clearing Date"),
                amount: signed("Amount (USD)", Direction::Out),
                payee: Some(Column::name("Merchant")),
                memo: Some(Column::name("Description")),
                ..base
            },
        ));
    }
    if has("date")
        && has("description")
        && has("amount")
        && (has("card member") || has("extended details") || has("appears on your statement as"))
    {
        return Some((
            CsvLayout::Amex,
            CsvMapping {
                date: Column::name("Date"),
                amount: signed("Amount", Direction::Out),
                payee: Some(Column::name("Description")),
                memo: has("extended details").then(|| Column::name("Extended Details")),
                fitid: has("reference").then(|| Column::name("Reference")),
                ..base
            },
        ));
    }
    if has("transaction date") && has("posted date") && has("debit") && has("credit") {
        return Some((
            CsvLayout::CapitalOneCard,
            CsvMapping {
                date: Column::name("Posted Date"),
                amount: AmountColumns::DebitCredit {
                    debit: Column::name("Debit"),
                    credit: Column::name("Credit"),
                },
                payee: Some(Column::name("Description")),
                ..base
            },
        ));
    }
    if has("transaction description")
        && has("transaction date")
        && has("transaction type")
        && has("transaction amount")
    {
        return Some((
            CsvLayout::CapitalOne360,
            CsvMapping {
                date: Column::name("Transaction Date"),
                amount: AmountColumns::AmountAndType {
                    amount: Column::name("Transaction Amount"),
                    type_column: Column::name("Transaction Type"),
                    out_values: vec!["Debit".into()],
                },
                payee: Some(Column::name("Transaction Description")),
                balance: has("balance").then(|| Column::name("Balance")),
                ..base
            },
        ));
    }
    if has("trans. date") && has("post date") && has("description") && has("amount") {
        return Some((
            CsvLayout::Discover,
            CsvMapping {
                date: Column::name("Post Date"),
                amount: signed("Amount", Direction::Out),
                payee: Some(Column::name("Description")),
                ..base
            },
        ));
    }
    None
}

/// Wells Fargo: no header, five columns: date, signed amount, `*`, check number,
/// description.
fn known_headerless(data: &[&Vec<String>]) -> Option<CsvMapping> {
    let first = data.first()?;
    let looks = first.len() == 5
        && row_has_date(&first[..1])
        && text::parse_amount(&first[1]).is_ok()
        && first[2] == "*";
    looks.then(|| CsvMapping {
        has_header: false,
        date: Column::Index(0),
        amount: AmountColumns::Signed {
            column: Column::Index(1),
            positive_is: Direction::In,
        },
        check_number: Some(Column::Index(3)),
        payee: Some(Column::Index(4)),
        ..CsvMapping::default()
    })
}

/// No header and not a layout we know: the first date-looking column, the first
/// amount-looking column after it, and the longest text column as the payee.
fn generic_headerless(data: &[&Vec<String>], notes: &mut Vec<String>) -> CsvMapping {
    let first = data.first().map(|r| r.as_slice()).unwrap_or_default();
    let date = first
        .iter()
        .position(|c| row_has_date(std::slice::from_ref(c)))
        .unwrap_or(0);
    let amount = (0..first.len())
        .find(|&i| i != date && text::parse_amount(&first[i]).is_ok())
        .unwrap_or(1);
    let payee = (0..first.len())
        .filter(|&i| i != date && i != amount)
        .max_by_key(|&i| first[i].len());
    let positive_is = majority_sign_direction(data.iter().filter_map(|r| r.get(amount)));
    notes
        .push("the file has no header row; columns were guessed from the first transaction".into());
    notes.push(sign_note(positive_is));
    CsvMapping {
        has_header: false,
        date: Column::Index(date as u32),
        amount: AmountColumns::Signed {
            column: Column::Index(amount as u32),
            positive_is,
        },
        payee: payee.map(|p| Column::Index(p as u32)),
        ..CsvMapping::default()
    }
}

/// A layout we don't know, read by its column names.
fn generic(
    headers: &[String],
    data: &[&Vec<String>],
    notes: &mut Vec<String>,
) -> Result<CsvMapping> {
    let lower: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    let find = |words: &[&str], avoid: &[&str]| -> Option<Column> {
        // Exact names first, then names containing a word.
        lower
            .iter()
            .position(|h| words.contains(&h.as_str()))
            .or_else(|| {
                lower.iter().position(|h| {
                    words.iter().any(|w| h.contains(w)) && !avoid.iter().any(|a| h.contains(a))
                })
            })
            .map(|i| Column::Name(headers[i].trim().to_owned()))
    };
    let date = find(
        &["posted date", "post date", "posting date", "date"],
        &["update"],
    )
    .ok_or_else(|| FeedError::parse("CSV", "no column looks like a date"))?;
    let debit = find(
        &[
            "debit",
            "debits",
            "withdrawal",
            "withdrawals",
            "money out",
            "paid out",
        ],
        &[],
    );
    let credit = find(
        &[
            "credit", "credits", "deposit", "deposits", "money in", "paid in",
        ],
        &["card"],
    );
    let amount = if let (Some(debit), Some(credit)) = (debit, credit) {
        notes.push("money out and money in are in separate columns".into());
        AmountColumns::DebitCredit { debit, credit }
    } else {
        let column = find(&["amount", "transaction amount", "value"], &[])
            .ok_or_else(|| FeedError::parse("CSV", "no column looks like an amount"))?;
        let idx = column.resolve(Some(headers)).unwrap_or(0);
        let positive_is = majority_sign_direction(data.iter().filter_map(|r| r.get(idx)));
        notes.push(sign_note(positive_is));
        AmountColumns::Signed {
            column,
            positive_is,
        }
    };
    let payee = find(
        &[
            "payee",
            "description",
            "name",
            "merchant",
            "details",
            "transaction description",
        ],
        &[],
    );
    let memo = find(&["memo", "notes", "note", "extended details"], &[])
        .filter(|m| Some(m) != payee.as_ref());
    let check_number = find(
        &[
            "check number",
            "check #",
            "check no",
            "check no.",
            "cheque number",
            "check",
        ],
        &[],
    );
    let fitid = find(
        &["transaction id", "reference", "reference number", "fitid"],
        &[],
    );
    let balance = find(&["balance", "running balance", "running bal."], &[]);
    notes.push("columns were matched by their names; check them before importing".into());
    Ok(CsvMapping {
        date,
        amount,
        payee,
        memo,
        check_number,
        fitid,
        balance,
        ..CsvMapping::default()
    })
}

/// Most transactions on any account are spending, so whichever sign most rows
/// carry is "money out".
fn majority_sign_direction<'a>(cells: impl Iterator<Item = &'a String>) -> Direction {
    let (mut pos, mut neg) = (0, 0);
    for c in cells {
        match text::parse_amount(c) {
            Ok(m) if m.is_positive() => pos += 1,
            Ok(m) if m.is_negative() => neg += 1,
            _ => {}
        }
    }
    if pos > neg {
        Direction::Out
    } else {
        Direction::In
    }
}

fn sign_note(d: Direction) -> String {
    match d {
        Direction::In => {
            "most amounts are negative, so negative was taken as money out (check this)".into()
        }
        Direction::Out => {
            "most amounts are positive, so positive was taken as money out, as card issuers write it (check this)".into()
        }
    }
}

/// Work out the date order: a four-digit first field means year first; a first
/// field over 12 means day first; a second field over 12 means month first;
/// otherwise US month-first, with a note.
fn detect_date_order<'a>(cells: impl Iterator<Item = &'a String>) -> (DateOrder, Option<String>) {
    let mut ymd = false;
    let mut day_first = false;
    let mut month_first = false;
    for c in cells {
        let nums: Vec<&str> = c
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .take(3)
            .collect();
        if nums.len() < 3 {
            if nums.len() == 1 && nums[0].len() == 8 {
                ymd = true;
            }
            continue;
        }
        if nums[0].len() == 4 {
            ymd = true;
            continue;
        }
        let a: u32 = nums[0].parse().unwrap_or(0);
        let b: u32 = nums[1].parse().unwrap_or(0);
        if a > 12 {
            day_first = true;
        }
        if b > 12 {
            month_first = true;
        }
    }
    if ymd {
        return (DateOrder::Ymd, None);
    }
    match (day_first, month_first) {
        (true, false) => (DateOrder::Dmy, Some("dates are day/month/year".into())),
        (false, true) => (DateOrder::Mdy, None),
        _ => (
            DateOrder::Mdy,
            Some("dates were read as month/day/year (US); no date proved it either way".into()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimiters() {
        assert_eq!(detect_delimiter("a,b,c\n1,2,3\n"), ',');
        assert_eq!(detect_delimiter("a;b;c\n1;\"2,5\";3\n"), ';');
        assert_eq!(detect_delimiter("a\tb\n1\t2\n"), '\t');
    }

    #[test]
    fn date_orders() {
        let v = |xs: &[&str]| xs.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            detect_date_order(v(&["2026-10-01"]).iter()).0,
            DateOrder::Ymd
        );
        assert_eq!(
            detect_date_order(v(&["10/13/2026"]).iter()).0,
            DateOrder::Mdy
        );
        assert_eq!(
            detect_date_order(v(&["13/10/2026"]).iter()).0,
            DateOrder::Dmy
        );
        let (o, note) = detect_date_order(v(&["01/02/2026"]).iter());
        assert_eq!(o, DateOrder::Mdy);
        assert!(note.is_some());
    }
}
