//! The company file's schema, as an ordered list of migrations.
//!
//! Shape: every list record and every transaction is stored whole, as JSON, in a
//! `doc` column, with the columns that queries filter, join or sort on pulled out
//! beside it. The ledger itself (`posting`) and payment applications
//! (`application`) are fully relational, because reports and open balances are
//! computed from them. The schema version is SQLite's `user_version`.
//!
//! Migrations only ever append. Never edit one that has shipped.

pub const MIGRATIONS: &[&str] = &[
    // 1: the books.
    r"
CREATE TABLE company (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    name                TEXT NOT NULL,
    doc                 TEXT NOT NULL,            -- CompanyInfo as JSON
    closing_date        TEXT,                     -- YYYY-MM-DD, NULL when the books are open
    created_at          TEXT NOT NULL,
    modified_at         TEXT NOT NULL
) STRICT;

CREATE TABLE account (
    id            TEXT PRIMARY KEY,
    parent_id     TEXT REFERENCES account(id),
    name          TEXT NOT NULL,
    full_name     TEXT NOT NULL COLLATE NOCASE UNIQUE,
    account_type  TEXT NOT NULL,
    number        TEXT,
    role          TEXT UNIQUE,
    is_active     INTEGER NOT NULL,
    sort_key      TEXT NOT NULL,                    -- type order + number + full name
    edit_seq      INTEGER NOT NULL,
    doc           TEXT NOT NULL
) STRICT;
CREATE INDEX account_parent ON account(parent_id);

-- Customers (and jobs), vendors, employees, other names: one namespace, as in QBD.
CREATE TABLE entity (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    parent_id   TEXT REFERENCES entity(id),
    name        TEXT NOT NULL,
    full_name   TEXT NOT NULL COLLATE NOCASE UNIQUE,
    is_active   INTEGER NOT NULL,
    edit_seq    INTEGER NOT NULL,
    doc         TEXT NOT NULL
) STRICT;
CREATE INDEX entity_kind ON entity(kind, full_name);
CREATE INDEX entity_parent ON entity(parent_id);

CREATE TABLE item (
    id          TEXT PRIMARY KEY,
    item_type   TEXT NOT NULL,
    parent_id   TEXT REFERENCES item(id),
    name        TEXT NOT NULL,
    full_name   TEXT NOT NULL COLLATE NOCASE UNIQUE,
    is_active   INTEGER NOT NULL,
    edit_seq    INTEGER NOT NULL,
    doc         TEXT NOT NULL
) STRICT;

-- The small lists share one table: classes, terms, payment methods, sales tax
-- codes, sales reps, customer/vendor/job types.
CREATE TABLE list_entry (
    id          TEXT PRIMARY KEY,
    list        TEXT NOT NULL,
    name        TEXT NOT NULL,
    full_name   TEXT NOT NULL,
    parent_id   TEXT REFERENCES list_entry(id),
    is_active   INTEGER NOT NULL,
    edit_seq    INTEGER NOT NULL,
    doc         TEXT NOT NULL,
    UNIQUE (list, full_name COLLATE NOCASE)
) STRICT;

CREATE TABLE txn (
    id                TEXT PRIMARY KEY,
    txn_type          TEXT NOT NULL,
    date              TEXT NOT NULL,
    number            TEXT,
    entity_id         TEXT REFERENCES entity(id),
    account_id        TEXT REFERENCES account(id),
    ar_ap_account_id  TEXT REFERENCES account(id),
    amount            INTEGER NOT NULL,             -- the form's total, minor units
    sales_tax         INTEGER NOT NULL DEFAULT 0,
    -- What is still owed on an invoice/bill/charge, or unused on a credit or an
    -- overpayment. NULL for types that are never open.
    open_balance      INTEGER,
    due_date          TEXT,
    is_void           INTEGER NOT NULL DEFAULT 0,
    is_pending        INTEGER NOT NULL DEFAULT 0,
    memo              TEXT,
    edit_seq          INTEGER NOT NULL,
    created_at        TEXT NOT NULL,
    modified_at       TEXT NOT NULL,
    created_by        TEXT,
    modified_by       TEXT,
    doc               TEXT NOT NULL                 -- the whole Txn, lines and all
) STRICT;
CREATE INDEX txn_date ON txn(date, txn_type);
CREATE INDEX txn_entity ON txn(entity_id, date);
CREATE INDEX txn_type_number ON txn(txn_type, number);
CREATE INDEX txn_open ON txn(ar_ap_account_id, entity_id) WHERE open_balance IS NOT NULL AND open_balance <> 0;

-- The ledger. Debit-positive. Each transaction's postings sum to zero (enforced
-- by the store before commit, and checked by `check_integrity`).
CREATE TABLE posting (
    id                INTEGER PRIMARY KEY,
    txn_id            TEXT NOT NULL REFERENCES txn(id) ON DELETE CASCADE,
    seq               INTEGER NOT NULL,
    date              TEXT NOT NULL,                -- copied from the txn for range scans
    txn_type          TEXT NOT NULL,
    account_id        TEXT NOT NULL REFERENCES account(id),
    amount            INTEGER NOT NULL,
    entity_id         TEXT REFERENCES entity(id),
    class_id          TEXT REFERENCES list_entry(id),
    item_id           TEXT REFERENCES item(id),
    line_id           TEXT,
    memo              TEXT,
    quantity          TEXT,                         -- decimal, inventory only
    cleared           TEXT NOT NULL DEFAULT 'Uncleared',
    reconciliation_id TEXT,
    UNIQUE (txn_id, seq)
) STRICT;
CREATE INDEX posting_account_date ON posting(account_id, date);
CREATE INDEX posting_entity_date ON posting(entity_id, date) WHERE entity_id IS NOT NULL;
CREATE INDEX posting_item ON posting(item_id, date) WHERE item_id IS NOT NULL;
CREATE INDEX posting_class ON posting(class_id) WHERE class_id IS NOT NULL;

-- Payments applied to open transactions, and credits used against them.
CREATE TABLE application (
    txn_id               TEXT NOT NULL REFERENCES txn(id) ON DELETE CASCADE,
    seq                  INTEGER NOT NULL,
    target_txn_id        TEXT NOT NULL REFERENCES txn(id),
    date                 TEXT NOT NULL,             -- the paying txn's date
    amount               INTEGER NOT NULL,
    discount             INTEGER NOT NULL,
    discount_account_id  TEXT REFERENCES account(id),
    credit_txn_id        TEXT REFERENCES txn(id),
    credit_amount        INTEGER NOT NULL,
    PRIMARY KEY (txn_id, seq)
) STRICT;
CREATE INDEX application_target ON application(target_txn_id);
CREATE INDEX application_credit ON application(credit_txn_id) WHERE credit_txn_id IS NOT NULL;

-- Deposit lines that move a payment out of Undeposited Funds.
CREATE TABLE deposit_link (
    deposit_txn_id  TEXT NOT NULL REFERENCES txn(id) ON DELETE CASCADE,
    line_id         TEXT NOT NULL,
    payment_txn_id  TEXT NOT NULL REFERENCES txn(id),
    PRIMARY KEY (deposit_txn_id, line_id)
) STRICT;
CREATE INDEX deposit_link_payment ON deposit_link(payment_txn_id);

-- Every save, void and delete, with the record before and after: QBD's audit trail.
CREATE TABLE audit (
    id          INTEGER PRIMARY KEY,
    at          TEXT NOT NULL,
    user_id     TEXT,
    record_kind TEXT NOT NULL,                       -- txn, account, entity, item, list, company
    record_id   TEXT NOT NULL,
    action      TEXT NOT NULL,                       -- create, update, void, delete
    before      TEXT,
    after       TEXT,
    note        TEXT
) STRICT;
CREATE INDEX audit_record ON audit(record_kind, record_id, id);
CREATE INDEX audit_at ON audit(at);

-- Where a record came from: QuickBooks ListID/TxnID and EditSequence, a bank's
-- FITID, an Amazon order. Makes imports idempotent and incremental.
CREATE TABLE external_ref (
    source         TEXT NOT NULL,                    -- e.g. 'qbd:<company file id>', 'ofx:<bank>:<acct>'
    kind           TEXT NOT NULL,                    -- the source's record type
    external_id    TEXT NOT NULL,
    local_kind     TEXT NOT NULL,                    -- txn, account, entity, item, list
    local_id       TEXT NOT NULL,
    edit_sequence  TEXT,
    hash           TEXT,
    synced_at      TEXT NOT NULL,
    PRIMARY KEY (source, kind, external_id)
) STRICT;
CREATE INDEX external_ref_local ON external_ref(local_kind, local_id);

CREATE TABLE reconciliation (
    id                 TEXT PRIMARY KEY,
    account_id         TEXT NOT NULL REFERENCES account(id),
    statement_date     TEXT NOT NULL,
    beginning_balance  INTEGER NOT NULL,
    ending_balance     INTEGER NOT NULL,
    finished_at        TEXT,
    finished_by        TEXT,
    doc                TEXT NOT NULL
) STRICT;
CREATE INDEX reconciliation_account ON reconciliation(account_id, statement_date);

CREATE TABLE attachment (
    id           TEXT PRIMARY KEY,
    record_kind  TEXT,                               -- what it is attached to, if anything yet
    record_id    TEXT,
    filename     TEXT NOT NULL,
    mime         TEXT NOT NULL,
    size         INTEGER NOT NULL,
    sha256       TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    created_by   TEXT,
    data         BLOB NOT NULL
) STRICT;
CREATE INDEX attachment_record ON attachment(record_kind, record_id);

-- Memorized reports and transactions, saved views, and other per-company
-- documents keyed by kind and name.
CREATE TABLE saved (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    name        TEXT NOT NULL,
    doc         TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    UNIQUE (kind, name)
) STRICT;

CREATE TABLE setting (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
) STRICT;
",
    // 2: account feeds (bank downloads, card downloads, Amazon orders…).
    r"
CREATE TABLE feed_connection (
    id           TEXT PRIMARY KEY,
    provider     TEXT NOT NULL,                      -- ofx-file, simplefin, ofx-direct, plaid, …
    name         TEXT NOT NULL,
    account_id   TEXT REFERENCES account(id),        -- the LibreBooks account it feeds
    doc          TEXT NOT NULL,                      -- provider settings (secrets live elsewhere)
    last_sync_at TEXT,
    last_error   TEXT
) STRICT;

-- One downloaded transaction, waiting to be matched or added (QBD's Bank Feeds).
CREATE TABLE feed_txn (
    id              TEXT PRIMARY KEY,
    connection_id   TEXT REFERENCES feed_connection(id),
    account_id      TEXT NOT NULL REFERENCES account(id),
    fitid           TEXT NOT NULL,                   -- the bank's id for it
    posted_date     TEXT NOT NULL,
    amount          INTEGER NOT NULL,                -- from the account holder's side: + money in
    payee           TEXT,
    memo            TEXT,
    check_number    TEXT,
    status          TEXT NOT NULL,                   -- new, matched, added, ignored
    matched_txn_id  TEXT REFERENCES txn(id),
    doc             TEXT NOT NULL,                   -- everything the source sent
    imported_at     TEXT NOT NULL,
    UNIQUE (account_id, fitid)
) STRICT;
CREATE INDEX feed_txn_status ON feed_txn(account_id, status, posted_date);

-- Renaming and categorising rules for downloaded transactions.
CREATE TABLE feed_rule (
    id        TEXT PRIMARY KEY,
    priority  INTEGER NOT NULL,
    name      TEXT NOT NULL,
    doc       TEXT NOT NULL
) STRICT;
",
    // 3: the bank-feed "downloaded" mark. QBD keeps it apart from the reconcile
    // check mark: a line matched to a download is not thereby reconciled.
    r"
ALTER TABLE posting ADD COLUMN downloaded INTEGER NOT NULL DEFAULT 0;
",
    // 4: order fulfilment. Lines entered against another transaction's lines
    // (a receipt's against a purchase order's, an invoice's against a sales
    // order's or an estimate's, a bill's against a receipt's), kept beside the
    // JSON so an order's status is one indexed query. Deposit lines keep their
    // own table. Backfilled from the docs; an order's open balance starts at
    // its amount and `orders::refresh_all` makes it exact on first open.
    r"
CREATE TABLE txn_link (
    txn_id          TEXT NOT NULL REFERENCES txn(id) ON DELETE CASCADE,
    line_id         TEXT NOT NULL,
    target_txn_id   TEXT NOT NULL REFERENCES txn(id),
    target_line_id  TEXT,
    PRIMARY KEY (txn_id, line_id)
) STRICT;
CREATE INDEX txn_link_target ON txn_link(target_txn_id);

INSERT OR IGNORE INTO txn_link (txn_id, line_id, target_txn_id, target_line_id)
SELECT t.id, json_extract(l.value, '$.id'), json_extract(l.value, '$.linked_txn_id'),
       json_extract(l.value, '$.linked_line_id')
FROM txn t, json_each(t.doc, '$.lines') l
WHERE t.txn_type IN ('ItemReceipt', 'Bill', 'Check', 'CreditCardCharge', 'Invoice', 'SalesReceipt')
  AND t.is_void = 0
  AND json_extract(l.value, '$.linked_txn_id') IS NOT NULL
  AND EXISTS (SELECT 1 FROM txn o WHERE o.id = json_extract(l.value, '$.linked_txn_id'));

UPDATE txn SET open_balance = amount
WHERE txn_type IN ('PurchaseOrder', 'SalesOrder', 'Estimate') AND is_void = 0;
",
];

/// The schema version that added order fulfilment (`txn_link`, open balances
/// on orders). Files from before it get their orders recomputed on open.
pub const ORDER_FULFILMENT_VERSION: i64 = 4;
