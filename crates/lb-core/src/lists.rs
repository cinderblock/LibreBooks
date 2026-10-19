//! The smaller lists: classes, terms, payment methods, sales tax codes, sales reps,
//! and customer/vendor/job types.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClassId, EntityId, PaymentMethodId, SalesRepId, SalesTaxCodeId, TermsId, TypeId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Class {
    pub id: ClassId,
    pub name: String,
    pub full_name: String,
    pub parent_id: Option<ClassId>,
    pub sublevel: u32,
    pub is_active: bool,
    pub edit_seq: i64,
}

/// Payment terms. Standard terms count days from the transaction date; date-driven
/// terms name a day of the month (QBD's two kinds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Terms {
    pub id: TermsId,
    pub name: String,
    pub is_active: bool,
    pub kind: TermsKind,
    pub edit_seq: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type")]
pub enum TermsKind {
    /// Net `due_days`; `discount_percent` off if paid within `discount_days`.
    Standard {
        due_days: u32,
        discount_days: Option<u32>,
        #[ts(type = "string | null")]
        discount_percent: Option<Decimal>,
    },
    /// Due on `day_of_month` (of the next month if issued within `cutoff_days`).
    DateDriven {
        day_of_month: u8,
        cutoff_days: u8,
        discount_day_of_month: Option<u8>,
        #[ts(type = "string | null")]
        discount_percent: Option<Decimal>,
    },
}

impl Terms {
    /// The due date for a transaction dated `date`.
    pub fn due_date(&self, date: jiff::civil::Date) -> jiff::civil::Date {
        match &self.kind {
            TermsKind::Standard { due_days, .. } => date
                .checked_add(jiff::Span::new().days(i64::from(*due_days)))
                .unwrap_or(date),
            TermsKind::DateDriven {
                day_of_month,
                cutoff_days,
                ..
            } => date_driven_due(date, *day_of_month, *cutoff_days),
        }
    }

    /// The last day an early-payment discount applies, if there is one.
    pub fn discount_date(&self, date: jiff::civil::Date) -> Option<jiff::civil::Date> {
        match &self.kind {
            TermsKind::Standard {
                discount_days: Some(d),
                discount_percent: Some(_),
                ..
            } => date.checked_add(jiff::Span::new().days(i64::from(*d))).ok(),
            TermsKind::DateDriven {
                discount_day_of_month: Some(d),
                discount_percent: Some(_),
                cutoff_days,
                ..
            } => Some(date_driven_due(date, *d, *cutoff_days)),
            _ => None,
        }
    }
}

/// QBD's date-driven rule: due on `day` of this month, or of next month if the
/// transaction falls within `cutoff` days of it (or after it).
fn date_driven_due(date: jiff::civil::Date, day: u8, cutoff: u8) -> jiff::civil::Date {
    let this_month = clamp_day(date.year(), date.month(), day);
    let threshold = this_month
        .checked_sub(jiff::Span::new().days(i64::from(cutoff)))
        .unwrap_or(this_month);
    if date < threshold {
        this_month
    } else {
        let next = date
            .first_of_month()
            .checked_add(jiff::Span::new().months(1))
            .unwrap_or(date);
        clamp_day(next.year(), next.month(), day)
    }
}

fn clamp_day(year: i16, month: i8, day: u8) -> jiff::civil::Date {
    let first = jiff::civil::Date::new(year, month, 1).expect("valid month");
    let last = first.last_of_month().day();
    let d = (day as i8).clamp(1, last);
    jiff::civil::Date::new(year, month, d).expect("clamped day")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PaymentMethod {
    pub id: PaymentMethodId,
    pub name: String,
    pub kind: PaymentMethodKind,
    pub is_active: bool,
    pub edit_seq: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PaymentMethodKind {
    Cash,
    Check,
    AmericanExpress,
    Discover,
    MasterCard,
    Visa,
    DebitCard,
    GiftCard,
    ECheck,
    Other,
    OtherCreditCard,
}

/// Taxable or not. QBD ships `Tax` and `Non`; companies add more for reporting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SalesTaxCode {
    pub id: SalesTaxCodeId,
    pub name: String,
    pub description: Option<String>,
    pub is_taxable: bool,
    pub is_active: bool,
    pub edit_seq: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SalesRep {
    pub id: SalesRepId,
    /// Initials, as QBD shows them on forms.
    pub initials: String,
    /// The employee, vendor or other name this rep is.
    pub entity_id: Option<EntityId>,
    pub is_active: bool,
    pub edit_seq: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TypeListKind {
    CustomerType,
    VendorType,
    JobType,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TypeEntry {
    pub id: TypeId,
    pub list: TypeListKind,
    pub name: String,
    pub full_name: String,
    pub parent_id: Option<TypeId>,
    pub is_active: bool,
    pub edit_seq: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn terms(kind: TermsKind) -> Terms {
        Terms {
            id: TermsId::new(),
            name: "t".into(),
            is_active: true,
            kind,
            edit_seq: 0,
        }
    }

    #[test]
    fn standard_terms() {
        let net30 = terms(TermsKind::Standard {
            due_days: 30,
            discount_days: None,
            discount_percent: None,
        });
        assert_eq!(net30.due_date(date(2026, 1, 15)), date(2026, 2, 14));
        let two_ten = terms(TermsKind::Standard {
            due_days: 30,
            discount_days: Some(10),
            discount_percent: Some(Decimal::TWO),
        });
        assert_eq!(
            two_ten.discount_date(date(2026, 1, 15)),
            Some(date(2026, 1, 25))
        );
        assert_eq!(net30.discount_date(date(2026, 1, 15)), None);
    }

    #[test]
    fn date_driven_terms() {
        // Due the 10th; issued within 5 days of it rolls to next month.
        let t = terms(TermsKind::DateDriven {
            day_of_month: 10,
            cutoff_days: 5,
            discount_day_of_month: None,
            discount_percent: None,
        });
        assert_eq!(t.due_date(date(2026, 3, 1)), date(2026, 3, 10));
        assert_eq!(t.due_date(date(2026, 3, 6)), date(2026, 4, 10));
        assert_eq!(t.due_date(date(2026, 3, 20)), date(2026, 4, 10));
        // Day 31 in a 30-day month clamps.
        let eom = terms(TermsKind::DateDriven {
            day_of_month: 31,
            cutoff_days: 0,
            discount_day_of_month: None,
            discount_percent: None,
        });
        assert_eq!(eom.due_date(date(2026, 4, 2)), date(2026, 4, 30));
        assert_eq!(eom.due_date(date(2026, 4, 30)), date(2026, 5, 31));
    }
}
