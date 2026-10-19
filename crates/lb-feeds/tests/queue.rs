//! The Bank Feeds queue against a real company file: importing (and not
//! importing twice), rules, matching, adding, undoing, connections, Amazon.

use jiff::civil::{Date, date};
use lb_core::*;
use lb_feeds::add::{self, AddRequest};
use lb_feeds::connections::{self, FeedConnection, ProviderSettings};
use lb_feeds::ingest::{self, IngestOptions};
use lb_feeds::matching::{self, MatchOptions};
use lb_feeds::queue::{self, FeedFilter, FeedStatus, ImportOptions};
use lb_feeds::rules::{self, Condition, FeedRule, RuleActions, TextField, TextOp};
use lb_feeds::statement::Direction;
use lb_feeds::{FeedError, FeedTxnId, Statement, StatementTxn, amazon, ofx};
use lb_store::{Actor, Books, accounts, balances, entities, txns};
use pretty_assertions::assert_eq;

fn me() -> Actor {
    Actor {
        user_id: Some("tester".into()),
        may_override_closing_date: false,
    }
}

struct Co {
    books: Books,
    checking: AccountId,
    visa: AccountId,
    utilities: AccountId,
    meals: AccountId,
    sales: AccountId,
    supplies: AccountId,
    comcast: EntityId,
    acme: EntityId,
}

fn company() -> Co {
    let books = Books::in_memory(&CompanyInfo {
        name: "Feeds Co".into(),
        ..CompanyInfo::default()
    })
    .unwrap();
    let a = |w: &mut lb_store::Writer<'_>, name: &str, t| {
        accounts::save(w, accounts::new_account(name, t)).map(|a| a.id)
    };
    let (checking, visa, utilities, meals, sales, supplies, comcast, acme) = books
        .write(&me(), |w| {
            Ok((
                a(w, "Checking", AccountType::Bank)?,
                a(w, "Visa", AccountType::CreditCard)?,
                a(w, "Utilities", AccountType::Expense)?,
                a(w, "Meals", AccountType::Expense)?,
                a(w, "Sales", AccountType::Income)?,
                a(w, "Office Supplies", AccountType::Expense)?,
                entities::save(w, Entity::blank(EntityKind::Vendor, "Comcast"))?.id,
                entities::save(w, Entity::blank(EntityKind::Customer, "Acme Corp"))?.id,
            ))
        })
        .unwrap();
    Co {
        books,
        checking,
        visa,
        utilities,
        meals,
        sales,
        supplies,
        comcast,
        acme,
    }
}

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{}/testdata/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn checking_statement() -> Statement {
    ofx::parse_statements(&fixture("ofx/checking-sgml-102.ofx"))
        .unwrap()
        .remove(0)
}

fn card_statement() -> Statement {
    ofx::parse_statements(&fixture("ofx/card-xml-220.qfx"))
        .unwrap()
        .remove(0)
}

fn statement(txns: &[(Date, i64, &str)]) -> Statement {
    let mut s = Statement::default();
    for (d, cents, payee) in txns {
        let mut t = StatementTxn::new(*d, Money(*cents));
        t.payee = Some((*payee).to_owned());
        s.transactions.push(t);
    }
    s.finish();
    s
}

fn ingest_into(co: &Co, account: AccountId, s: &Statement) -> ingest::IngestReport {
    lb_feeds::write(&co.books, &me(), |w| {
        ingest::ingest(w, account, s, &IngestOptions::default())
    })
    .unwrap()
}

fn waiting(co: &Co, account: AccountId) -> Vec<queue::FeedTxn> {
    lb_feeds::read(&co.books, |c| {
        queue::query(
            c,
            &FeedFilter {
                account_id: Some(account),
                statuses: vec![FeedStatus::New],
                ..FeedFilter::default()
            },
        )
    })
    .unwrap()
}

fn by_payee(list: &[queue::FeedTxn], payee: &str) -> FeedTxnId {
    list.iter()
        .find(|t| t.payee.as_deref().is_some_and(|p| p.contains(payee)))
        .unwrap_or_else(|| panic!("no {payee}"))
        .id
}

fn cleared_in(co: &Co, account: AccountId, txn: TxnId) -> ClearedStatus {
    let (_, lines) = co
        .books
        .read(|c| balances::register(c, account, None, None))
        .unwrap();
    lines.iter().find(|l| l.txn_id == txn).unwrap().cleared
}

/// A check written by hand in Checking.
fn write_check(
    co: &Co,
    d: Date,
    cents: i64,
    number: Option<&str>,
    payee: Option<EntityId>,
) -> TxnId {
    let mut t = Txn::new(TxnType::Check, d);
    t.account_id = Some(co.checking);
    t.number = number.map(str::to_owned);
    t.entity_id = payee;
    t.lines.push(TxnLine::account(co.utilities, Money(cents)));
    co.books.write(&me(), |w| txns::save(w, t)).unwrap().txn.id
}

fn deposit(co: &Co, d: Date, cents: i64) -> TxnId {
    let mut t = Txn::new(TxnType::Deposit, d);
    t.account_id = Some(co.checking);
    t.lines
        .push(TxnLine::deposit(co.sales, Money(cents)).with_entity(co.acme));
    co.books.write(&me(), |w| txns::save(w, t)).unwrap().txn.id
}

#[test]
fn importing_twice_adds_nothing_the_second_time() {
    let co = company();
    let s = checking_statement();
    let first = ingest_into(&co, co.checking, &s);
    assert_eq!(first.import.imported.len(), 5);
    assert_eq!(first.import.duplicates, 0);
    assert_eq!(first.waiting, 5);
    let second = ingest_into(&co, co.checking, &s);
    assert_eq!(second.import.imported.len(), 0);
    assert_eq!(second.import.duplicates, 5);

    // Synthesised ids dedupe too (a CSV of the same account, imported twice).
    let csv = lb_feeds::csvfile::detect(&fixture("csv/chase-checking.csv"))
        .unwrap()
        .preview;
    assert_eq!(ingest_into(&co, co.visa, &csv).import.imported.len(), 5);
    assert_eq!(ingest_into(&co, co.visa, &csv).import.duplicates, 5);
    // The same fitid in another account is another transaction.
    assert_eq!(ingest_into(&co, co.visa, &s).import.imported.len(), 5);
}

#[test]
fn start_date_and_reversed_signs() {
    let co = company();
    let opts = IngestOptions {
        import: ImportOptions {
            not_before: Some(date(2026, 10, 5)),
            reverse_signs: true,
            ..ImportOptions::default()
        },
        ..IngestOptions::default()
    };
    let r = lb_feeds::write(&co.books, &me(), |w| {
        ingest::ingest(w, co.checking, &checking_statement(), &opts)
    })
    .unwrap();
    assert_eq!(r.import.before_start, 2);
    assert_eq!(r.import.imported.len(), 3);
    let fee = waiting(&co, co.checking)
        .into_iter()
        .find(|t| t.fitid == "202610140005")
        .unwrap();
    assert_eq!(fee.amount, Money(1200));
    assert_eq!(fee.source, "file");
}

#[test]
fn feeds_go_into_registers_only() {
    let co = company();
    let err = lb_feeds::write(&co.books, &me(), |w| {
        ingest::ingest(
            w,
            co.utilities,
            &checking_statement(),
            &IngestOptions::default(),
        )
    })
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Utilities (Expense) can't have a feed; feeds go into bank"),
        "{err}"
    );
}

#[test]
fn query_ignore_summary_and_undo() {
    let co = company();
    ingest_into(&co, co.checking, &checking_statement());
    let list = waiting(&co, co.checking);
    let fee = by_payee(&list, "SERVICE FEE");
    let n = lb_feeds::write(&co.books, &me(), |w| queue::ignore(w, &[fee, fee])).unwrap();
    assert_eq!(n, 1);
    assert_eq!(waiting(&co, co.checking).len(), 4);

    let found = lb_feeds::read(&co.books, |c| {
        queue::query(
            c,
            &FeedFilter {
                text: Some("comcast".into()),
                ..FeedFilter::default()
            },
        )
    })
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].amount, Money(-8999));
    let by_amount = lb_feeds::read(&co.books, |c| {
        queue::query(
            c,
            &FeedFilter {
                amount: Some(Money(8999)),
                descending: true,
                ..FeedFilter::default()
            },
        )
    })
    .unwrap();
    assert_eq!(by_amount[0].id, found[0].id);

    let summary = lb_feeds::read(&co.books, queue::summary).unwrap();
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].account_id, co.checking);
    assert_eq!((summary[0].new, summary[0].ignored), (4, 1));
    assert_eq!(summary[0].latest_posted, Some(date(2026, 10, 14)));

    let back = lb_feeds::write(&co.books, &me(), |w| queue::undo(w, fee)).unwrap();
    assert_eq!(back.status, FeedStatus::New);
    assert_eq!(waiting(&co, co.checking).len(), 5);
}

#[test]
fn downloads_match_what_was_entered_by_hand() {
    let co = company();
    // Entered before the download: the check a day early, the deposit, and
    // Comcast paid by check without a number the day before.
    let check = write_check(&co, date(2026, 10, 2), 120_000, Some("1041"), None);
    let dep = deposit(&co, date(2026, 10, 1), 250_000);
    let cable = write_check(&co, date(2026, 10, 6), 8999, None, Some(co.comcast));
    // Something that matches nothing: same amount as the fee, but a deposit.
    deposit(&co, date(2026, 10, 14), 1200);

    let r = ingest_into(&co, co.checking, &checking_statement());
    let matched: Vec<TxnId> = r.matched.iter().map(|m| m.txn_id).collect();
    assert_eq!(matched.len(), 3, "{r:?}");
    for t in [check, dep, cable] {
        assert!(matched.contains(&t));
        assert_eq!(cleared_in(&co, co.checking, t), ClearedStatus::Cleared);
    }
    assert_eq!(r.waiting, 2);
    let left: Vec<String> = waiting(&co, co.checking)
        .into_iter()
        .filter_map(|t| t.payee)
        .collect();
    assert_eq!(left, vec!["SQ *CAFÉ BLEU #0412", "MONTHLY SERVICE FEE"]);

    // Undoing a match unclears the register line and frees it.
    let feed = r
        .matched
        .iter()
        .find(|m| m.txn_id == check)
        .unwrap()
        .feed_txn_id;
    lb_feeds::write(&co.books, &me(), |w| queue::undo(w, feed)).unwrap();
    assert_eq!(
        cleared_in(&co, co.checking, check),
        ClearedStatus::Uncleared
    );
    let f = lb_feeds::read(&co.books, |c| queue::get(c, feed)).unwrap();
    let cands = lb_feeds::read(&co.books, |c| {
        matching::candidates(c, &f, MatchOptions::default())
    })
    .unwrap();
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].txn_id, check);
    assert_eq!(cands[0].score, 97);
    assert!(cands[0].reasons.contains(&"check number 1041".to_owned()));
}

#[test]
fn ambiguous_matches_wait_for_a_person() {
    let co = company();
    let a = write_check(&co, date(2026, 10, 7), 8999, None, None);
    let b = write_check(&co, date(2026, 10, 7), 8999, None, None);
    let r = ingest_into(&co, co.checking, &checking_statement());
    assert!(r.matched.iter().all(|m| m.txn_id != a && m.txn_id != b));
    let comcast = by_payee(&waiting(&co, co.checking), "COMCAST");
    let f = lb_feeds::read(&co.books, |c| queue::get(c, comcast)).unwrap();
    let cands = lb_feeds::read(&co.books, |c| {
        matching::candidates(c, &f, MatchOptions::default())
    })
    .unwrap();
    assert_eq!(cands.len(), 2);
    assert!(cands.iter().all(|c| c.strong));

    // Two downloads that both want one register line: neither takes it.
    let co = company();
    let only = write_check(&co, date(2026, 10, 3), 5000, None, None);
    let twins = statement(&[
        (date(2026, 10, 3), -5000, "TRANSFER"),
        (date(2026, 10, 3), -5000, "TRANSFER"),
    ]);
    let r = ingest_into(&co, co.checking, &twins);
    assert_eq!(r.matched.len(), 0);
    assert_eq!(cleared_in(&co, co.checking, only), ClearedStatus::Uncleared);

    // A person picks: the first gets it, and the second no longer sees it.
    let ids = r.import.imported.clone();
    lb_feeds::write(&co.books, &me(), |w| matching::accept(w, ids[0], only)).unwrap();
    assert_eq!(cleared_in(&co, co.checking, only), ClearedStatus::Cleared);
    let second = lb_feeds::read(&co.books, |c| queue::get(c, ids[1])).unwrap();
    let cands = lb_feeds::read(&co.books, |c| {
        matching::candidates(c, &second, MatchOptions::default())
    })
    .unwrap();
    assert_eq!(cands.len(), 0);
    let err = lb_feeds::write(&co.books, &me(), |w| matching::accept(w, ids[1], only)).unwrap_err();
    assert!(err.to_string().contains("already cleared"), "{err}");
}

#[test]
fn accepting_a_match_checks_the_amount() {
    let co = company();
    let other = write_check(&co, date(2026, 10, 7), 9000, None, None);
    let r = ingest_into(&co, co.checking, &checking_statement());
    let comcast = by_payee(&waiting(&co, co.checking), "COMCAST");
    assert_eq!(r.matched.len(), 0);
    let err =
        lb_feeds::write(&co.books, &me(), |w| matching::accept(w, comcast, other)).unwrap_err();
    assert_eq!(
        err.to_string(),
        "the amounts differ: the bank says -89.99, the register says -90.00"
    );
    let visa_charge = {
        let mut t = Txn::new(TxnType::CreditCardCharge, date(2026, 10, 7));
        t.account_id = Some(co.visa);
        t.lines.push(TxnLine::account(co.utilities, Money(8999)));
        co.books.write(&me(), |w| txns::save(w, t)).unwrap().txn.id
    };
    let err = lb_feeds::write(&co.books, &me(), |w| {
        matching::accept(w, comcast, visa_charge)
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "that transaction isn't in this account");
}

#[test]
fn adding_makes_checks_deposits_and_card_transactions_that_balance() {
    let co = company();
    ingest_into(&co, co.checking, &checking_statement());
    ingest_into(&co, co.visa, &card_statement());
    let bank = waiting(&co, co.checking);
    let card = waiting(&co, co.visa);

    let add_one = |id: FeedTxnId, req: AddRequest| {
        lb_feeds::write(&co.books, &me(), |w| add::add(w, id, &req)).unwrap()
    };
    let get_txn = |id: TxnId| co.books.read(|c| txns::get(c, id)).unwrap();

    // Money out of the bank: a check, numbered from the bank's check number.
    let o = add_one(
        by_payee(&bank, "CHECK 1041"),
        AddRequest {
            entity_id: Some(co.comcast),
            account_id: Some(co.utilities),
            ..AddRequest::default()
        },
    );
    assert_eq!(o.txn_type, TxnType::Check);
    let t = get_txn(o.txn_id);
    assert_eq!(t.number.as_deref(), Some("1041"));
    assert_eq!(t.entity_id, Some(co.comcast));
    assert_eq!(t.amount, Money(120_000));
    assert_eq!(t.date, date(2026, 10, 3));
    assert_eq!(
        cleared_in(&co, co.checking, o.txn_id),
        ClearedStatus::Cleared
    );

    // Money in: a deposit, the name on the line.
    let o = add_one(
        by_payee(&bank, "ACME PAYROLL"),
        AddRequest {
            entity_id: Some(co.acme),
            account_id: Some(co.sales),
            ..AddRequest::default()
        },
    );
    assert_eq!(o.txn_type, TxnType::Deposit);
    let t = get_txn(o.txn_id);
    assert_eq!(t.entity_id, None);
    assert_eq!(t.lines[0].entity_id, Some(co.acme));
    assert_eq!(t.lines[0].kind, LineKind::DepositLine);
    // No memo chosen: the bank's memo is kept (the payee is the chosen name).
    assert_eq!(t.memo.as_deref(), Some("DIRECT DEP PPD ID: 9999999999"));

    // A card purchase: a charge; with no name chosen the bank's text is the memo.
    let o = add_one(
        by_payee(&card, "AMZN"),
        AddRequest {
            account_id: Some(co.supplies),
            ..AddRequest::default()
        },
    );
    assert_eq!(o.txn_type, TxnType::CreditCardCharge);
    assert_eq!(
        get_txn(o.txn_id).memo.as_deref(),
        Some("AMZN Mktp US*2K4AB1CD2 Amzn.com/bill WA")
    );
    // A card refund: a credit.
    let o = add_one(
        by_payee(&card, "Café"),
        AddRequest {
            account_id: Some(co.supplies),
            memo: Some("returned a book".into()),
            ..AddRequest::default()
        },
    );
    assert_eq!(o.txn_type, TxnType::CreditCardCredit);
    assert_eq!(get_txn(o.txn_id).memo.as_deref(), Some("returned a book"));
    // A card payment from checking: a credit against the bank.
    let o = add_one(
        by_payee(&card, "PAYMENT THANK YOU"),
        AddRequest {
            account_id: Some(co.checking),
            ..AddRequest::default()
        },
    );
    assert_eq!(o.txn_type, TxnType::CreditCardCredit);

    // The books balance, and the registers say what the bank says.
    assert_eq!(co.books.check_integrity().unwrap(), Vec::<String>::new());
    let sums = co.books.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&co.utilities], Money(120_000));
    assert_eq!(sums[&co.sales], Money(-250_000));
    assert_eq!(sums[&co.supplies], Money(4217 - 1999));
    // Visa (credit-normal): owes 42.17, refunded 19.99, paid 500 → debit 477.82.
    assert_eq!(sums[&co.visa], Money(-4217 + 1999 + 50_000));
    assert_eq!(
        sums[&co.checking],
        Money(-120_000 + 250_000 - 50_000),
        "checking: check, deposit, card payment"
    );

    // Nothing to add twice, and no account means no add.
    let err = lb_feeds::write(&co.books, &me(), |w| {
        add::add(w, by_payee(&bank, "CHECK 1041"), &AddRequest::default())
    })
    .unwrap_err();
    assert!(err.to_string().contains("already added"), "{err}");
    let err = lb_feeds::write(&co.books, &me(), |w| {
        add::add(w, by_payee(&bank, "COMCAST"), &AddRequest::default())
    })
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "choose an account for COMCAST CABLE COMM on 2026-10-07"
    );
}

#[test]
fn add_all_adds_what_it_can_and_reports_the_rest() {
    let co = company();
    let r = ingest_into(&co, co.checking, &checking_statement());
    let ids = r.import.imported.clone();
    // A rule supplies an account for Comcast only.
    lb_feeds::write(&co.books, &me(), |w| {
        rules::save(
            w,
            FeedRule::new(
                "Cable",
                vec![Condition::Text {
                    field: TextField::Payee,
                    op: TextOp::StartsWith,
                    value: "comcast".into(),
                }],
                RuleActions {
                    entity_id: Some(co.comcast),
                    account_id: Some(co.utilities),
                    ..RuleActions::default()
                },
            ),
        )?;
        rules::apply_all(w, None)
    })
    .unwrap();
    let before = co
        .books
        .read(|c| txns::query(c, &lb_store::TxnFilter::default()))
        .unwrap()
        .len();
    let out = lb_feeds::write(&co.books, &me(), |w| add::add_all(w, &ids, None)).unwrap();
    assert_eq!(out.added.len(), 1);
    assert_eq!(out.failed.len(), 4);
    assert!(out.failed[0].message.starts_with("choose an account"));
    let after = co
        .books
        .read(|c| txns::query(c, &lb_store::TxnFilter::default()))
        .unwrap()
        .len();
    assert_eq!(after, before + 1, "failures leave nothing behind");

    // With an account for everything (the person's choice on "Add All"), the rest go in.
    let rest: Vec<FeedTxnId> = out.failed.iter().map(|f| f.feed_txn_id).collect();
    let all = lb_feeds::write(&co.books, &me(), |w| {
        add::add_all(
            w,
            &rest,
            Some(&AddRequest {
                account_id: Some(co.meals),
                ..AddRequest::default()
            }),
        )
    })
    .unwrap();
    assert_eq!((all.added.len(), all.failed.len()), (4, 0));
    assert_eq!(waiting(&co, co.checking).len(), 0);
    assert_eq!(co.books.check_integrity().unwrap(), Vec::<String>::new());

    // Undo an added one: its transaction goes away.
    let added = all.added[0].clone();
    lb_feeds::write(&co.books, &me(), |w| queue::undo(w, added.feed_txn_id)).unwrap();
    assert!(
        co.books
            .read(|c| txns::find(c, added.txn_id))
            .unwrap()
            .is_none()
    );
    assert_eq!(waiting(&co, co.checking).len(), 1);
}

#[test]
fn rules_rename_categorise_and_auto_add() {
    let co = company();
    lb_feeds::write(&co.books, &me(), |w| {
        // Lower priority first: the specific rule wins over the catch-all.
        let mut square = FeedRule::new(
            "Square coffee",
            vec![
                Condition::Text {
                    field: TextField::Payee,
                    op: TextOp::Regex,
                    value: r"^sq \*caf".into(),
                },
                Condition::AmountRange {
                    min: None,
                    max: Some(Money(2000)),
                },
            ],
            RuleActions {
                rename_to: Some("Café Bleu".into()),
                account_id: Some(co.meals),
                auto_add: true,
                ..RuleActions::default()
            },
        );
        square.priority = 1;
        rules::save(w, square)?;
        let mut catch_all = FeedRule::new(
            "Everything out",
            vec![Condition::Direction {
                direction: Direction::Out,
            }],
            RuleActions {
                account_id: Some(co.utilities),
                ..RuleActions::default()
            },
        );
        catch_all.priority = 2;
        rules::save(w, catch_all)?;
        Ok(())
    })
    .unwrap();

    let r = ingest_into(&co, co.checking, &checking_statement());
    // The café was added by its rule; nothing in the register looked like it.
    assert_eq!(r.auto_added.len(), 1, "{r:?}");
    let cafe = co
        .books
        .read(|c| txns::get(c, r.auto_added[0].txn_id))
        .unwrap();
    assert_eq!(cafe.txn_type, TxnType::Check);
    assert_eq!(cafe.lines[0].account_id, Some(co.meals));
    assert_eq!(cafe.amount, Money(450));

    // The catch-all suggested Utilities for the other outflows, without adding.
    let left = waiting(&co, co.checking);
    let fee = left
        .iter()
        .find(|t| t.payee.as_deref() == Some("MONTHLY SERVICE FEE"))
        .unwrap();
    let s = fee.suggestion.as_ref().unwrap();
    assert_eq!(s.account_id, Some(co.utilities));
    assert!(!s.auto_add);
    let payroll = left
        .iter()
        .find(|t| t.payee.as_deref() == Some("ACME PAYROLL"))
        .unwrap();
    assert_eq!(payroll.suggestion, None, "money in matches no rule");

    // Auto-add never duplicates something already entered.
    let co2 = company();
    lb_feeds::write(&co2.books, &me(), |w| {
        rules::save(
            w,
            FeedRule::new(
                "Fees",
                vec![Condition::AmountEquals {
                    amount: Money(1200),
                }],
                RuleActions {
                    account_id: Some(co2.utilities),
                    auto_add: true,
                    ..RuleActions::default()
                },
            ),
        )
    })
    .unwrap();
    // A weak candidate 9 days away is enough to hold it back.
    write_check(&co2, date(2026, 10, 5), 1200, None, None);
    let r = ingest_into(&co2, co2.checking, &checking_statement());
    assert!(r.auto_added.is_empty() && r.matched.is_empty());
}

#[test]
fn bad_rules_are_refused() {
    let co = company();
    let attempt = |r: FeedRule| {
        lb_feeds::write(&co.books, &me(), |w| rules::save(w, r))
            .unwrap_err()
            .to_string()
    };
    let text = |v: &str, op| Condition::Text {
        field: TextField::PayeeOrMemo,
        op,
        value: v.into(),
    };
    assert!(
        attempt(FeedRule::new(
            "x",
            vec![text("([", TextOp::Regex)],
            RuleActions::default()
        ))
        .starts_with("the pattern \"([\" isn't valid")
    );
    assert_eq!(
        attempt(FeedRule::new("x", vec![], RuleActions::default())),
        "a rule needs at least one condition"
    );
    assert_eq!(
        attempt(FeedRule::new(
            "x",
            vec![text("a", TextOp::Contains)],
            RuleActions {
                auto_add: true,
                ..RuleActions::default()
            }
        )),
        "to add transactions automatically, the rule must choose an account"
    );
    assert_eq!(
        attempt(FeedRule::new(
            "x",
            vec![Condition::AmountRange {
                min: Some(Money(500)),
                max: Some(Money(100))
            }],
            RuleActions::default()
        )),
        "the amount range's minimum is above its maximum"
    );
    // Reordering and deleting.
    let (a, b) = lb_feeds::write(&co.books, &me(), |w| {
        let a = rules::save(
            w,
            FeedRule::new(
                "A",
                vec![text("a", TextOp::Contains)],
                RuleActions::default(),
            ),
        )?;
        let b = rules::save(
            w,
            FeedRule::new(
                "B",
                vec![text("b", TextOp::Contains)],
                RuleActions::default(),
            ),
        )?;
        Ok((a.id, b.id))
    })
    .unwrap();
    let names = |co: &Co| -> Vec<String> {
        lb_feeds::read(&co.books, rules::list)
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect()
    };
    assert_eq!(names(&co), vec!["A", "B"]);
    lb_feeds::write(&co.books, &me(), |w| rules::reorder(w, &[b, a])).unwrap();
    assert_eq!(names(&co), vec!["B", "A"]);
    lb_feeds::write(&co.books, &me(), |w| rules::delete(w, a)).unwrap();
    assert_eq!(names(&co), vec!["B"]);
}

#[test]
fn history_remembers_how_a_payee_was_added() {
    let co = company();
    ingest_into(
        &co,
        co.checking,
        &statement(&[(date(2026, 9, 7), -8999, "COMCAST CABLE COMM 09/07")]),
    );
    let id = waiting(&co, co.checking)[0].id;
    lb_feeds::write(&co.books, &me(), |w| {
        add::add(
            w,
            id,
            &AddRequest {
                entity_id: Some(co.comcast),
                account_id: Some(co.utilities),
                ..AddRequest::default()
            },
        )
    })
    .unwrap();
    // Next month's bill (the date in the payee differs) is offered the same.
    let r = ingest_into(
        &co,
        co.checking,
        &statement(&[(date(2026, 10, 7), -9150, "COMCAST CABLE COMM 10/07")]),
    );
    assert_eq!(r.waiting, 1);
    let next = &waiting(&co, co.checking)[0];
    let s = next.suggestion.as_ref().unwrap();
    assert!(s.from_history);
    assert_eq!(s.entity_id, Some(co.comcast));
    assert_eq!(s.account_id, Some(co.utilities));
    assert!(!s.auto_add);
    // With the suggestion, adding needs no choices.
    let o = lb_feeds::write(&co.books, &me(), |w| {
        add::add(w, next.id, &AddRequest::default())
    })
    .unwrap();
    let t = co.books.read(|c| txns::get(c, o.txn_id)).unwrap();
    assert_eq!(t.entity_id, Some(co.comcast));
}

#[test]
fn connections_are_saved_checked_and_remembered() {
    let co = company();
    let conn = lb_feeds::write(&co.books, &me(), |w| {
        connections::save(
            w,
            FeedConnection::new(
                "Everyday checking",
                Some(co.checking),
                ProviderSettings::File { csv_mapping: None },
            ),
        )
    })
    .unwrap();
    assert_eq!(conn.edit_seq, 1);
    assert_eq!(conn.match_days, 10);

    // Stale edits and unfeedable accounts are refused.
    let mut stale = conn.clone();
    stale.edit_seq = 0;
    stale.id = conn.id;
    let err = lb_feeds::write(&co.books, &me(), |w| connections::save(w, stale)).unwrap_err();
    assert!(
        matches!(err, FeedError::NotFound { .. } | FeedError::Store(_)),
        "{err:?}"
    );
    let mut wrong = conn.clone();
    wrong.account_id = Some(co.utilities);
    assert!(lb_feeds::write(&co.books, &me(), |w| connections::save(w, wrong)).is_err());

    // The first file through a connection teaches it the account's fingerprint.
    let s = checking_statement();
    let r = lb_feeds::write(&co.books, &me(), |w| {
        ingest::ingest_for_connection(w, &conn, std::slice::from_ref(&s))
    })
    .unwrap();
    assert_eq!(r.import.imported.len(), 5);
    let conn = lb_feeds::read(&co.books, |c| connections::get(c, conn.id)).unwrap();
    assert_eq!(
        conn.account_key.as_deref(),
        Some(s.account.account_key.as_str())
    );
    let found = lb_feeds::read(&co.books, |c| {
        connections::by_account_key(c, &s.account.account_key)
    })
    .unwrap();
    assert_eq!(found.map(|f| f.id), Some(conn.id));
    let fed = waiting(&co, co.checking);
    assert!(fed.iter().all(|t| t.connection_id == Some(conn.id)));

    // A multi-account file only feeds the statement with that fingerprint.
    let multi = ofx::parse_statements(&fixture("ofx/multi-account.ofx")).unwrap();
    let r = lb_feeds::write(&co.books, &me(), |w| {
        ingest::ingest_for_connection(w, &conn, &multi)
    })
    .unwrap();
    assert_eq!(
        r.import.imported.len(),
        0,
        "none of those accounts is this one"
    );
    let fresh = FeedConnection::new(
        "New",
        Some(co.checking),
        ProviderSettings::File { csv_mapping: None },
    );
    let err = lb_feeds::write(&co.books, &me(), |w| {
        ingest::ingest_for_connection(w, &fresh, &multi)
    })
    .unwrap_err();
    assert!(err.to_string().contains("several accounts"));

    // Deleting a connection keeps its history.
    lb_feeds::write(&co.books, &me(), |w| {
        connections::delete(w, conn.id, conn.edit_seq)
    })
    .unwrap();
    let kept = waiting(&co, co.checking);
    assert_eq!(kept.len(), 5);
    assert!(kept.iter().all(|t| t.connection_id.is_none()));
    assert_eq!(
        lb_feeds::read(&co.books, connections::list).unwrap().len(),
        0
    );
}

#[test]
fn amazon_orders_explain_card_charges() {
    let co = company();
    let orders = amazon::parse_order_history(&fixture("amazon/Retail.OrderHistory.1.csv")).unwrap();
    assert_eq!(orders.len(), 2, "the cancelled order is left out");
    assert_eq!(orders[0].total, Money(4217));
    assert_eq!(orders[0].shipments.len(), 1);
    assert_eq!(orders[1].total, Money(4999));
    assert_eq!(orders[1].shipments.len(), 2);
    assert_eq!(orders[1].items[1].quantity, 2);
    assert_eq!(orders[0].payment.as_deref(), Some("Visa - 7890"));

    let r = ingest_into(
        &co,
        co.visa,
        &statement(&[
            (date(2026, 9, 20), -4217, "AMZN Mktp US*2K4AB1CD2"),
            (date(2026, 9, 20), -4217, "DELTA AIR LINES"),
            (date(2026, 10, 2), -1999, "AMAZON.COM*AB12CD34E"),
            (date(2026, 10, 3), -500, "AMZN Mktp US*ZZ"),
            (date(2026, 10, 6), -3000, "AMZN Mktp US*XY"),
        ]),
    );
    assert_eq!(r.import.imported.len(), 5);
    let feed = waiting(&co, co.visa);
    let matches = amazon::match_charges(&feed, &orders);
    let pairs: Vec<(String, Option<Date>)> = matches
        .iter()
        .map(|m| (m.order_id.clone(), m.ship_date))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("111-0000001-0000001".into(), None),
            ("111-0000002-0000002".into(), Some(date(2026, 10, 1))),
            ("111-0000002-0000002".into(), Some(date(2026, 10, 5))),
        ]
    );
    assert_eq!(
        matches[0].memo,
        "Amazon 111-0000001-0000001: USB-C Cable, 2 m; Grid Notebook, A5"
    );
    assert_eq!(
        matches[2].memo,
        "Amazon 111-0000002-0000002: Printer Paper, 500 sheets"
    );

    let n = lb_feeds::write(&co.books, &me(), |w| amazon::annotate(w, &matches, &orders)).unwrap();
    assert_eq!(n, 3);
    let t = lb_feeds::read(&co.books, |c| queue::get(c, matches[0].feed_txn_id)).unwrap();
    assert_eq!(t.memo.as_deref(), Some(matches[0].memo.as_str()));
    assert_eq!(t.raw["amazon"]["order_id"], "111-0000001-0000001");
    assert_eq!(
        t.raw["amazon"]["link"],
        "https://www.amazon.com/gp/your-account/order-details?orderID=111-0000001-0000001"
    );
    // Adding it carries the order onto the charge.
    let o = lb_feeds::write(&co.books, &me(), |w| {
        add::add(
            w,
            t.id,
            &AddRequest {
                account_id: Some(co.supplies),
                ..AddRequest::default()
            },
        )
    })
    .unwrap();
    let charge = co.books.read(|c| txns::get(c, o.txn_id)).unwrap();
    assert!(
        charge
            .memo
            .as_deref()
            .unwrap()
            .contains("Amazon 111-0000001-0000001: USB-C Cable")
    );
}
