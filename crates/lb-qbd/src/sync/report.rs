//! What a sync did, for people: counts per QBD record type, every record that
//! failed and why, every figure where LibreBooks' posting rules disagree with
//! what QBD stored, and the gaps the bridge leaves.

use std::collections::BTreeMap;

use lb_core::Money;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::entity::QbdEntity;

/// A whole sync.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SyncReport {
    /// `qbd:ein-…`: which QBD company the books are bound to.
    pub source_id: String,
    pub started_at: Option<jiff::Timestamp>,
    pub finished_at: Option<jiff::Timestamp>,
    /// Requests sent to the bridge, where the source counts them.
    #[ts(type = "number | null")]
    pub requests: Option<u64>,
    /// Watermarks ignored: everything was listed again.
    pub full: bool,
    /// Limited or date-restricted: watermarks were left alone and no sweep ran.
    pub partial_run: bool,
    pub entities: Vec<EntityReport>,
    pub issues: Vec<SyncIssue>,
    pub mismatches: Vec<Mismatch>,
    pub gaps: Vec<Gap>,
    /// QBD fields LibreBooks has no home for (`Customer.PriceLevelRef`), with how
    /// many records carried them. Transactions keep theirs in `extra`; these are
    /// from list records and sub-records.
    pub unmapped_fields: BTreeMap<String, u32>,
    /// Records the import had to create because the bridge doesn't list them
    /// (sales tax codes, customer types…).
    pub created_lists: Vec<String>,
    /// Set when the sync stopped early (the bridge went away); what finished is
    /// kept, and the next sync carries on from there.
    pub aborted: Option<String>,
}

impl SyncReport {
    pub fn entity_mut(&mut self, entity: QbdEntity) -> &mut EntityReport {
        if let Some(i) = self.entities.iter().position(|e| e.entity == entity) {
            return &mut self.entities[i];
        }
        self.entities.push(EntityReport {
            entity,
            ..EntityReport::default()
        });
        self.entities.last_mut().expect("just pushed")
    }

    pub fn entity(&self, entity: QbdEntity) -> Option<&EntityReport> {
        self.entities.iter().find(|e| e.entity == entity)
    }

    pub fn errors(&self) -> impl Iterator<Item = &SyncIssue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    /// One line per record type, for logs and the CLI.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        for e in &self.entities {
            s.push_str(&format!(
                "{:<17} listed {:>5}  created {:>5}  updated {:>5}  unchanged {:>5}  skipped {:>3}  failed {:>3}  partial {:>3}  deleted {:>3}\n",
                e.entity.name(),
                e.listed,
                e.created,
                e.updated,
                e.unchanged,
                e.skipped,
                e.failed,
                e.partial,
                e.deleted + e.deactivated
            ));
        }
        s.push_str(&format!(
            "{} errors, {} warnings, {} mismatches with QuickBooks' figures",
            self.errors().count(),
            self.issues
                .iter()
                .filter(|i| i.severity == Severity::Warning)
                .count(),
            self.mismatches.len()
        ));
        if let Some(a) = &self.aborted {
            s.push_str(&format!("\nstopped early: {a}"));
        }
        s
    }
}

/// Counts for one QBD record type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntityReport {
    pub entity: QbdEntity,
    /// Rows the bridge listed (changed since the watermark, or all).
    pub listed: u32,
    pub created: u32,
    pub updated: u32,
    /// Same `EditSequence` as last time: not fetched again.
    pub unchanged: u32,
    /// Deliberately not imported (see the issue for why).
    pub skipped: u32,
    pub failed: u32,
    /// Imported, but a link to another transaction is missing for now; retried
    /// next sync.
    pub partial: u32,
    /// Deleted in QBD, so deleted here (sweep).
    pub deleted: u32,
    /// Deleted in QBD but still used here, so made inactive instead.
    pub deactivated: u32,
    pub watermark_before: Option<String>,
    pub watermark_after: Option<String>,
}

impl Default for EntityReport {
    fn default() -> Self {
        EntityReport {
            entity: QbdEntity::Account,
            listed: 0,
            created: 0,
            updated: 0,
            unchanged: 0,
            skipped: 0,
            failed: 0,
            partial: 0,
            deleted: 0,
            deactivated: 0,
            watermark_before: None,
            watermark_after: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Severity {
    /// The record wasn't imported (or a whole step failed).
    Error,
    /// Imported or skipped, but something about it needs a person's eye.
    Warning,
    Info,
}

/// Something about one QBD record (or one step).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SyncIssue {
    pub severity: Severity,
    pub entity: QbdEntity,
    /// `ListID` / `TxnID`.
    pub qbd_id: Option<String>,
    /// `Check 8090 (2026-09-01)`, `Account "Utilities:Internet"`.
    pub record: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum MismatchKind {
    /// The form's total.
    Total,
    SalesTax,
    /// What is still owed on it or unused.
    OpenBalance,
}

/// A figure QBD stored that LibreBooks computes differently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Mismatch {
    pub entity: QbdEntity,
    pub qbd_id: String,
    pub record: String,
    pub kind: MismatchKind,
    pub qbd: Money,
    pub ours: Money,
}

/// Something the bridge can't give us, and how often the data pointed at it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Gap {
    /// A QBD transaction type (`CreditCardCharge`) or list (`PayrollItemWage`).
    pub qbd_type: String,
    /// Links from imported records to records of this type.
    pub links: u32,
    pub note: String,
}

/// Where a sync is, for a progress bar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SyncProgress {
    pub entity: QbdEntity,
    pub phase: SyncPhase,
    #[ts(type = "number")]
    pub done: usize,
    #[ts(type = "number")]
    pub total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SyncPhase {
    Listing,
    Importing,
    Sweeping,
    Checking,
}
