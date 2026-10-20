//! What a report is, once run: one generic shape for every report.
//!
//! The UI renders a [`Report`] without knowing which report it is: a title block,
//! typed columns, and rows that say what they are (a section header, a line of
//! data, a subtotal, a total) and how far to indent. Amounts are cents, already
//! in the sign a person expects (income and expenses positive). Every row, and
//! every amount that summarises something, carries a [`Drill`] target, QBD's
//! QuickZoom: double-click an amount and see what makes it up.

use jiff::civil::Date;
use lb_core::{EntityId, Money, TxnId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::request::{Basis, ReportId, ReportRequest};

/// A finished report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Report {
    pub id: ReportId,
    /// "Profit & Loss", or a memorized report's own title.
    pub title: String,
    pub company_name: String,
    /// The date line as QBD prints it: "January 1 through October 19, 2026",
    /// "As of October 19, 2026", "All Transactions".
    pub subtitle: String,
    /// `None` for reports where the basis means nothing (aging, lists).
    pub basis: Option<Basis>,
    pub generated_at: jiff::Timestamp,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    /// Things a reader should know (a setting that was ignored, a comparison
    /// that could not be made).
    pub notes: Vec<String>,
}

/// What a column holds, so the UI can format and align it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ColumnKind {
    Text,
    /// Cents.
    Money,
    Date,
    /// A decimal string (quantities, rates, days).
    Number,
    /// A decimal string, already multiplied by 100 ("12.5" is 12.5%).
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// One column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Column {
    /// Stable within the report (`"label"`, `"c0"`, `"debit"`…), for the UI to
    /// key on and remember widths by.
    pub id: String,
    pub label: String,
    pub kind: ColumnKind,
    pub align: Align,
    /// A heading spanning several columns ("Oct 2026" over "Actual", "% of
    /// Income"), when the report has one.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl Column {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: ColumnKind) -> Column {
        let align = match kind {
            ColumnKind::Text => Align::Left,
            ColumnKind::Date => Align::Center,
            ColumnKind::Money | ColumnKind::Number | ColumnKind::Percent => Align::Right,
        };
        Column {
            id: id.into(),
            label: label.into(),
            kind,
            align,
            group: None,
        }
    }

    #[must_use]
    pub fn grouped(mut self, group: Option<String>) -> Column {
        self.group = group;
        self
    }
}

/// What a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum RowKind {
    /// A section or parent heading: "Income", "Checking", "Acme Corp".
    Header,
    Data,
    /// "Total Income", "Total Checking", "Gross Profit".
    Subtotal,
    /// The report's bottom line: "Net Income", "TOTAL ASSETS".
    Total,
    /// A blank line between sections.
    Spacer,
}

/// A cell's value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", content = "value")]
pub enum Value {
    Empty,
    Text(String),
    Money(Money),
    Date(Date),
    /// A decimal string.
    Number(String),
    /// A decimal string, already multiplied by 100.
    Percent(String),
}

/// One cell: a value and, for amounts, where QuickZoom goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Cell {
    pub value: Value,
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill: Option<Drill>,
}

impl Cell {
    pub const EMPTY: Cell = Cell {
        value: Value::Empty,
        drill: None,
    };

    pub fn text(s: impl Into<String>) -> Cell {
        Cell {
            value: Value::Text(s.into()),
            drill: None,
        }
    }

    pub fn opt_text(s: Option<impl Into<String>>) -> Cell {
        s.map_or(Cell::EMPTY, Cell::text)
    }

    pub fn money(m: Money) -> Cell {
        Cell {
            value: Value::Money(m),
            drill: None,
        }
    }

    pub fn date(d: Date) -> Cell {
        Cell {
            value: Value::Date(d),
            drill: None,
        }
    }

    pub fn opt_date(d: Option<Date>) -> Cell {
        d.map_or(Cell::EMPTY, Cell::date)
    }

    pub fn number(s: impl Into<String>) -> Cell {
        Cell {
            value: Value::Number(s.into()),
            drill: None,
        }
    }

    pub fn percent(s: Option<String>) -> Cell {
        s.map_or(Cell::EMPTY, |s| Cell {
            value: Value::Percent(s),
            drill: None,
        })
    }

    #[must_use]
    pub fn drill(mut self, drill: Option<Drill>) -> Cell {
        self.drill = drill;
        self
    }

    /// The amount in the cell, if it holds one.
    pub fn as_money(&self) -> Option<Money> {
        match self.value {
            Value::Money(m) => Some(m),
            _ => None,
        }
    }

    /// The text in the cell, if it holds some.
    pub fn as_text(&self) -> Option<&str> {
        match &self.value {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// One row. `cells` has one entry per column, in column order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Row {
    pub kind: RowKind,
    /// Indent level of the first column (0 = flush left).
    pub depth: u32,
    pub cells: Vec<Cell>,
    /// Where double-clicking the row goes (the transaction on a detail line, the
    /// account's detail on a summary line).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill: Option<Drill>,
}

impl Row {
    pub fn new(kind: RowKind, depth: u32, cells: Vec<Cell>) -> Row {
        Row {
            kind,
            depth,
            cells,
            drill: None,
        }
    }

    pub fn spacer(width: usize) -> Row {
        Row::new(RowKind::Spacer, 0, vec![Cell::EMPTY; width])
    }

    #[must_use]
    pub fn drill(mut self, drill: Option<Drill>) -> Row {
        self.drill = drill;
        self
    }

    /// The first cell's text (the row's label in most reports).
    pub fn label(&self) -> Option<&str> {
        self.cells.first().and_then(Cell::as_text)
    }
}

/// QuickZoom: where double-clicking goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum Drill {
    /// Open the transaction.
    Txn { txn_id: TxnId },
    /// Run this report: an account and a date range (with the column's name,
    /// class or item) on a summary amount becomes its transaction detail.
    Report { request: Box<ReportRequest> },
    /// Open the name (customer, vendor, employee).
    Name { entity_id: EntityId },
}

impl Report {
    /// The rows whose first cell reads `label` (tests and the UI's "find in
    /// report").
    pub fn rows_labelled<'a>(&'a self, label: &'a str) -> impl Iterator<Item = &'a Row> + 'a {
        self.rows.iter().filter(move |r| r.label() == Some(label))
    }

    /// The first row labelled `label`.
    pub fn row(&self, label: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.label() == Some(label))
    }

    /// The index of the column with this id.
    pub fn column_index(&self, id: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.id == id)
    }

    /// The amount in column `column_id` of the first row labelled `label` that
    /// has one there (a balance sheet's "Accounts Receivable" heading sits above
    /// the account of the same name).
    pub fn amount(&self, label: &str, column_id: &str) -> Option<Money> {
        let i = self.column_index(column_id)?;
        self.rows_labelled(label)
            .find_map(|r| r.cells.get(i).and_then(Cell::as_money))
    }
}
