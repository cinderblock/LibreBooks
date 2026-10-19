//! Shared helpers: the fixture company, a fresh in-memory book, balances by name.

use std::path::{Path, PathBuf};

use lb_core::{Account, CompanyInfo, Money, Txn, TxnId};
use lb_qbd::sync::{SyncOptions, SyncReport, sync};
use lb_qbd::testing::FixtureSource;
use lb_qbd::{QbdEntity, QbdSource};
use lb_store::{Books, accounts, balances, codec, extref};

pub fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

pub fn larkspur() -> FixtureSource {
    FixtureSource::load_dir(&testdata().join("larkspur")).expect("fixtures load")
}

pub fn books() -> Books {
    Books::in_memory(&CompanyInfo::default()).expect("in-memory books")
}

pub async fn run<S: QbdSource>(books: &Books, src: &S, options: &SyncOptions) -> SyncReport {
    sync(books, src, options, &|_| {}).await.expect("sync runs")
}

/// A sync that must not report any errors.
pub async fn run_clean<S: QbdSource>(books: &Books, src: &S, options: &SyncOptions) -> SyncReport {
    let report = run(books, src, options).await;
    let errors: Vec<_> = report.errors().collect();
    assert!(
        errors.is_empty(),
        "unexpected errors:\n{errors:#?}\n{}",
        report.summary()
    );
    report
}

pub fn account(books: &Books, full_name: &str) -> Account {
    books
        .read(|c| accounts::by_full_name(c, full_name))
        .unwrap()
        .unwrap_or_else(|| panic!("no account {full_name}"))
}

/// An account's all-time balance in its natural sign, as QBD shows it.
pub fn balance(books: &Books, full_name: &str) -> Money {
    let a = account(books, full_name);
    let sums = books.read(|c| balances::sums(c, None, None)).unwrap();
    a.account_type
        .natural(sums.get(&a.id).copied().unwrap_or(Money::ZERO))
}

pub fn m(s: &str) -> Money {
    s.parse().unwrap()
}

/// The local transaction imported from a QBD `TxnID`.
pub fn txn(books: &Books, entity: QbdEntity, qbd_id: &str) -> Txn {
    let source = lb_qbd::sync::bound_source(books).unwrap().unwrap();
    let r = books
        .read(|c| extref::get(c, &source, entity.name(), qbd_id))
        .unwrap()
        .unwrap_or_else(|| panic!("{qbd_id} not imported"));
    let id: TxnId = codec::parse_id(&r.local_id).unwrap();
    books.read(|c| lb_store::txns::get(c, id)).unwrap()
}

pub fn open_balance(books: &Books, id: TxnId) -> Option<Money> {
    lb_qbd::sync::import::open_balance(books, id).unwrap()
}
