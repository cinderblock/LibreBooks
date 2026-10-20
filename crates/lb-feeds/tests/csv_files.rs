//! CSV exports from the common US banks and card issuers, detected and read,
//! plus layouts nobody planned for.

use jiff::civil::date;
use lb_core::Money;
use lb_feeds::csvfile::{self, AmountColumns, Column, CsvLayout, CsvMapping, Direction};
use lb_feeds::text::DateOrder;
use pretty_assertions::assert_eq;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/testdata/csv/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// (date, cents, payee) of each transaction, for compact assertions.
fn rows(s: &lb_feeds::Statement) -> Vec<(String, i64, String)> {
    s.transactions
        .iter()
        .map(|t| {
            (
                t.posted.to_string(),
                t.amount.minor(),
                t.payee.clone().unwrap_or_default(),
            )
        })
        .collect()
}

#[test]
fn every_known_layout_is_recognised() {
    let cases = [
        ("chase-checking.csv", CsvLayout::ChaseChecking),
        ("chase-card.csv", CsvLayout::ChaseCard),
        ("bofa-checking.csv", CsvLayout::BankOfAmericaChecking),
        ("bofa-card.csv", CsvLayout::BankOfAmericaCard),
        ("amex.csv", CsvLayout::Amex),
        ("apple-card.csv", CsvLayout::AppleCard),
        ("capital-one-card.csv", CsvLayout::CapitalOneCard),
        ("capital-one-360.csv", CsvLayout::CapitalOne360),
        ("wells-fargo.csv", CsvLayout::WellsFargo),
        ("discover.csv", CsvLayout::Discover),
    ];
    for (file, layout) in cases {
        let d = csvfile::detect(&fixture(file)).unwrap();
        assert_eq!(d.layout, layout, "{file}");
        assert!(d.confident, "{file}");
        assert!(!d.preview.transactions.is_empty(), "{file}");
        assert!(
            d.preview.warnings.is_empty(),
            "{file}: {:?}",
            d.preview.warnings
        );
    }
}

#[test]
fn chase_checking() {
    let d = csvfile::detect(&fixture("chase-checking.csv")).unwrap();
    let s = &d.preview;
    assert_eq!(
        rows(s),
        vec![
            ("2026-10-14".into(), -1200, "MONTHLY SERVICE FEE".into()),
            (
                "2026-10-07".into(),
                -8999,
                "COMCAST CABLE COMM 800-555-0100 CA       10/07".into()
            ),
            (
                "2026-10-05".into(),
                -450,
                "SQ *CAFE BLEU #0412 OAKLAND CA".into()
            ),
            ("2026-10-03".into(), -120_000, "CHECK 1041".into()),
            (
                "2026-10-01".into(),
                250_000,
                "ACME PAYROLL      PPD ID: 9999999999".into()
            ),
        ]
    );
    assert_eq!(s.transactions[3].check_number.as_deref(), Some("1041"));
    // Newest first: the ending balance is the top row's.
    assert_eq!(s.ledger_balance, Some(Money(519_851)));
    assert_eq!(s.balance_date, Some(date(2026, 10, 14)));
    assert!(s.transactions.iter().all(|t| t.fitid_synthesized));
    assert_eq!(s.transactions[0].raw["Type"], "FEE_TRANSACTION");
}

#[test]
fn chase_card_with_bom() {
    let s = csvfile::detect(&fixture("chase-card.csv")).unwrap().preview;
    assert_eq!(
        rows(&s),
        vec![
            ("2026-10-13".into(), -1800, "NETFLIX.COM".into()),
            (
                "2026-10-11".into(),
                25_000,
                "AUTOMATIC PAYMENT - THANK".into()
            ),
            ("2026-10-10".into(), -4512, "SHELL OIL 57444".into()),
            ("2026-10-03".into(), 6499, "REI #12 BERKELEY".into()),
        ]
    );
}

#[test]
fn bank_of_america_checking_skips_its_summary() {
    let d = csvfile::detect(&fixture("bofa-checking.csv")).unwrap();
    assert_eq!(d.mapping.skip_rows, 5);
    assert_eq!(
        d.headers,
        vec!["Date", "Description", "Amount", "Running Bal."]
    );
    let s = d.preview;
    assert_eq!(
        s.transactions.len(),
        4,
        "the balance-only row is not a transaction"
    );
    assert_eq!(s.transactions[0].amount, Money(250_000));
    assert_eq!(s.transactions[1].amount, Money(-120_000));
    assert_eq!(s.ledger_balance, Some(Money(519_851)));
}

#[test]
fn bank_of_america_card_uses_its_reference_numbers() {
    let s = csvfile::detect(&fixture("bofa-card.csv")).unwrap().preview;
    assert_eq!(s.transactions[0].fitid, "24431066287000000000011");
    assert!(!s.transactions[0].fitid_synthesized);
    assert_eq!(s.transactions[0].memo.as_deref(), Some("866-579-7172 CA"));
    assert_eq!(s.transactions[1].amount, Money(25_000));
}

#[test]
fn amex_writes_charges_positive() {
    let s = csvfile::detect(&fixture("amex.csv")).unwrap().preview;
    assert_eq!(
        rows(&s),
        vec![
            (
                "2026-10-12".into(),
                -2341,
                "UBER *TRIP HELP.UBER.COM".into()
            ),
            (
                "2026-10-11".into(),
                50_000,
                "AUTOPAY PAYMENT - THANK YOU".into()
            ),
            (
                "2026-10-09".into(),
                -125_000,
                "DELTA AIR LINES ATLANTA".into()
            ),
            (
                "2026-10-09".into(),
                -125_000,
                "DELTA AIR LINES ATLANTA".into()
            ),
        ]
    );
    // Its references (with the leading apostrophe Excel needs removed) are the ids,
    // so the two identical flights stay two.
    assert_eq!(s.transactions[0].fitid, "320262850123456781");
    assert_ne!(s.transactions[2].fitid, s.transactions[3].fitid);
    // A multi-line quoted field.
    assert_eq!(
        s.transactions[0].memo.as_deref(),
        Some("UBER TRIP\nHELP.UBER.COM CA")
    );
}

#[test]
fn apple_card_uses_merchant_and_clearing_date() {
    let s = csvfile::detect(&fixture("apple-card.csv")).unwrap().preview;
    assert_eq!(s.transactions[0].payee.as_deref(), Some("Apple Services"));
    assert_eq!(s.transactions[0].posted, date(2026, 10, 13));
    assert_eq!(s.transactions[0].amount, Money(-999));
    assert_eq!(
        s.transactions[1].amount,
        Money(20_000),
        "payments are money in"
    );
}

#[test]
fn capital_one_debit_and_credit_columns() {
    let d = csvfile::detect(&fixture("capital-one-card.csv")).unwrap();
    assert_eq!(d.mapping.date_order, DateOrder::Ymd);
    assert_eq!(
        rows(&d.preview),
        vec![
            ("2026-10-13".into(), -2100, "GITHUB INC".into()),
            (
                "2026-10-11".into(),
                30_000,
                "CAPITAL ONE AUTOPAY PYMT".into()
            ),
            ("2026-10-10".into(), -5418, "TRADER JOE S #123".into()),
        ]
    );
}

#[test]
fn capital_one_360_amount_and_type_two_digit_years() {
    let s = csvfile::detect(&fixture("capital-one-360.csv"))
        .unwrap()
        .preview;
    assert_eq!(
        rows(&s),
        vec![
            ("2026-10-01".into(), 137, "Interest Paid".into()),
            (
                "2026-10-05".into(),
                -50_000,
                "Withdrawal to CHECKING XXXXXXX0000".into()
            ),
            (
                "2026-10-15".into(),
                100_000,
                "Deposit from ACME PAYROLL".into()
            ),
        ]
    );
    assert_eq!(s.ledger_balance, Some(Money(250_137)));
}

#[test]
fn wells_fargo_has_no_header() {
    let d = csvfile::detect(&fixture("wells-fargo.csv")).unwrap();
    assert!(!d.mapping.has_header);
    assert_eq!(d.headers, Vec::<String>::new());
    let s = d.preview;
    assert_eq!(s.transactions.len(), 3);
    assert_eq!(s.transactions[1].check_number.as_deref(), Some("1041"));
    assert_eq!(s.transactions[1].payee.as_deref(), Some("CHECK # 1041"));
    assert_eq!(s.transactions[2].amount, Money(-8999));
}

#[test]
fn discover_writes_purchases_positive() {
    let s = csvfile::detect(&fixture("discover.csv")).unwrap().preview;
    let amounts: Vec<i64> = s.transactions.iter().map(|t| t.amount.minor()).collect();
    assert_eq!(amounts, vec![15_000, -8743, 1000]);
}

#[test]
fn generic_semicolons_day_first_decimal_commas_1252() {
    let d = csvfile::detect(&fixture("generic-semicolon-dmy.csv")).unwrap();
    assert_eq!(d.layout, CsvLayout::Generic);
    assert!(!d.confident);
    assert_eq!(d.mapping.delimiter, ';');
    assert_eq!(d.mapping.date_order, DateOrder::Dmy);
    assert_eq!(
        d.mapping.amount,
        AmountColumns::DebitCredit {
            debit: Column::Name("Money Out".into()),
            credit: Column::Name("Money In".into()),
        }
    );
    let s = &d.preview;
    assert_eq!(
        rows(s),
        vec![
            ("2026-10-13".into(), -1250, "Café de Flore".into()),
            ("2026-10-14".into(), 200_000, "Salary".into()),
            ("2026-10-02".into(), -100_000, "Rent".into()),
        ]
    );
    assert_eq!(s.transactions[0].memo.as_deref(), Some("Lunch"));
    // Out of order: the balance is the latest date's.
    assert_eq!(s.ledger_balance, Some(Money(298_750)));
    assert!(s.warnings.is_empty(), "the Total footer is skipped quietly");
}

#[test]
fn generic_tab_separated_guesses_the_sign() {
    let d = csvfile::detect(&fixture("generic-tab.tsv")).unwrap();
    assert_eq!(d.mapping.delimiter, '\t');
    assert_eq!(
        d.mapping.amount,
        AmountColumns::Signed {
            column: Column::Name("Amount".into()),
            positive_is: Direction::In,
        }
    );
    assert_eq!(d.mapping.memo, Some(Column::Name("Notes".into())));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("negative was taken as money out"))
    );
    assert_eq!(d.preview.transactions[0].amount, Money(150_000));
}

#[test]
fn a_saved_mapping_survives_reordered_columns() {
    let mapping = CsvMapping {
        date: Column::Name("When".into()),
        amount: AmountColumns::Signed {
            column: Column::Name("Value".into()),
            positive_is: Direction::Out,
        },
        payee: Some(Column::Name("Who".into())),
        ..CsvMapping::default()
    };
    let a = b"When,Who,Value\n10/02/2026,Shop,5.00\n";
    let b = b"Value,When,Extra,Who\n5.00,10/02/2026,x,Shop\n";
    let sa = csvfile::parse(a, &mapping).unwrap();
    let sb = csvfile::parse(b, &mapping).unwrap();
    assert_eq!(sa.transactions[0].amount, Money(-500));
    assert_eq!(rows(&sa), rows(&sb));
    // The synthesised ids agree too, so the two files dedupe against each other.
    assert_eq!(sa.transactions[0].fitid, sb.transactions[0].fitid);
}

#[test]
fn a_mapping_that_does_not_fit_says_so() {
    let mapping = CsvMapping::default();
    let err = csvfile::parse(b"Posted,Text,Value\n10/02/2026,x,1\n", &mapping).unwrap_err();
    assert_eq!(
        err.to_string(),
        "CSV could not be read: the date column (\"Date\") isn't in this file"
    );
    let bad = b"Date,Description,Amount\n10/02/2026,x,lots\n10/32/2026,y,1.00\nTotal,,1.00\n";
    let s = csvfile::parse(bad, &mapping).unwrap();
    assert_eq!(s.transactions.len(), 0);
    assert_eq!(s.warnings.len(), 2, "{:?}", s.warnings);
}

#[test]
fn empty_and_dateless_files() {
    assert!(csvfile::detect(b"").is_err());
    assert!(csvfile::detect(b"a,b\nc,d\n").is_err());
}
