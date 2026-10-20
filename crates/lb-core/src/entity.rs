//! Names: customers (and their jobs), vendors, employees and other names.
//!
//! QBD keeps four name lists but requires a name to be unique across all of them,
//! because a transaction line's "Name" column can hold any of them. LibreBooks
//! keeps one table with a kind, and the same uniqueness rule over full names.
//! Jobs are customers with a parent, `Customer:Job:Sub-job`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{AccountId, EntityId, SalesRepId, SalesTaxCodeId, TermsId, TypeId};
use crate::money::Money;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum EntityKind {
    Customer,
    Vendor,
    Employee,
    OtherName,
}

impl EntityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            EntityKind::Customer => "Customer",
            EntityKind::Vendor => "Vendor",
            EntityKind::Employee => "Employee",
            EntityKind::OtherName => "OtherName",
        }
    }

    pub fn parse(s: &str) -> Option<EntityKind> {
        Some(match s {
            "Customer" => EntityKind::Customer,
            "Vendor" => EntityKind::Vendor,
            "Employee" => EntityKind::Employee,
            "OtherName" => EntityKind::OtherName,
            _ => return None,
        })
    }
}

/// A postal address as QBD stores it: five free lines, then structured fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Address {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub note: Option<String>,
}

impl Address {
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(String::is_empty)
            && self.city.is_none()
            && self.state.is_none()
            && self.postal_code.is_none()
            && self.country.is_none()
    }

    /// The address as printed on a form, one line per row.
    pub fn block(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .lines
            .iter()
            .filter(|l| !l.is_empty())
            .cloned()
            .collect();
        let mut last = String::new();
        if let Some(c) = &self.city {
            last.push_str(c);
        }
        if let Some(s) = &self.state {
            if !last.is_empty() {
                last.push_str(", ");
            }
            last.push_str(s);
        }
        if let Some(z) = &self.postal_code {
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(z);
        }
        if !last.is_empty() {
            out.push(last);
        }
        if let Some(c) = &self.country {
            out.push(c.clone());
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum JobStatus {
    None,
    Pending,
    Awarded,
    InProgress,
    Closed,
    NotAwarded,
}

/// A name in any of the four lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    /// The name's own part: `Phase 1`.
    pub name: String,
    /// `Acme Corp:Phase 1`. Unique across every kind.
    pub full_name: String,
    /// Jobs: the customer (or job) above. Other kinds: always `None`.
    pub parent_id: Option<EntityId>,
    pub sublevel: u32,
    pub is_active: bool,

    pub company_name: Option<String>,
    pub salutation: Option<String>,
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub job_title: Option<String>,
    /// "Print on check as" for vendors.
    pub print_as: Option<String>,
    pub bill_address: Address,
    pub ship_address: Address,
    pub phone: Option<String>,
    pub alt_phone: Option<String>,
    pub mobile: Option<String>,
    pub fax: Option<String>,
    pub email: Option<String>,
    pub cc_email: Option<String>,
    pub website: Option<String>,
    pub contact: Option<String>,
    pub alt_contact: Option<String>,
    pub notes: Option<String>,

    /// Our account number with them (vendors) or theirs with us (customers).
    pub account_number: Option<String>,
    pub terms_id: Option<TermsId>,
    pub credit_limit: Option<Money>,
    pub type_id: Option<TypeId>,
    pub sales_rep_id: Option<SalesRepId>,
    pub sales_tax_code_id: Option<SalesTaxCodeId>,
    pub tax_id: Option<String>,
    /// Vendors: eligible for a 1099.
    pub is_1099: bool,
    /// Vendors: the expense account pre-filled on their bills and checks.
    pub default_expense_account_id: Option<AccountId>,
    /// Customers: a price level as a percent markup/markdown.
    #[ts(type = "string | null")]
    pub price_level_percent: Option<Decimal>,

    /// Jobs.
    pub job_status: Option<JobStatus>,
    pub job_start_date: Option<jiff::civil::Date>,
    pub job_projected_end: Option<jiff::civil::Date>,
    pub job_end_date: Option<jiff::civil::Date>,
    pub job_description: Option<String>,
    pub job_type_id: Option<TypeId>,

    /// Employees: hire/release dates and the like that QBD keeps outside payroll.
    pub hired_date: Option<jiff::civil::Date>,
    pub released_date: Option<jiff::civil::Date>,

    /// User-defined fields, keyed by the field's label.
    #[ts(type = "Record<string, string>")]
    pub custom_fields: serde_json::Map<String, serde_json::Value>,
    #[ts(type = "number")]
    pub edit_seq: i64,
}

impl Entity {
    /// A blank name of a kind, for forms and tests.
    pub fn blank(kind: EntityKind, name: &str) -> Entity {
        Entity {
            id: EntityId::new(),
            kind,
            name: name.to_owned(),
            full_name: name.to_owned(),
            parent_id: None,
            sublevel: 0,
            is_active: true,
            company_name: None,
            salutation: None,
            first_name: None,
            middle_name: None,
            last_name: None,
            job_title: None,
            print_as: None,
            bill_address: Address::default(),
            ship_address: Address::default(),
            phone: None,
            alt_phone: None,
            mobile: None,
            fax: None,
            email: None,
            cc_email: None,
            website: None,
            contact: None,
            alt_contact: None,
            notes: None,
            account_number: None,
            terms_id: None,
            credit_limit: None,
            type_id: None,
            sales_rep_id: None,
            sales_tax_code_id: None,
            tax_id: None,
            is_1099: false,
            default_expense_account_id: None,
            price_level_percent: None,
            job_status: None,
            job_start_date: None,
            job_projected_end: None,
            job_end_date: None,
            job_description: None,
            job_type_id: None,
            hired_date: None,
            released_date: None,
            custom_fields: serde_json::Map::new(),
            edit_seq: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_block_reads_like_qbd() {
        let a = Address {
            lines: vec!["John Petro".into(), "1305 McKenzie Avenue".into()],
            city: Some("Los Altos".into()),
            state: Some("CA".into()),
            postal_code: Some("94024".into()),
            country: None,
            note: None,
        };
        assert_eq!(
            a.block(),
            vec!["John Petro", "1305 McKenzie Avenue", "Los Altos, CA 94024"]
        );
        assert!(Address::default().is_empty());
    }
}
