//! OFX, QFX and QBO files as banks actually send them: SGML and XML, CRLF and
//! LF, BOMs, Windows-1252, missing closing tags, odd dates and amounts, several
//! accounts in one file, errors instead of statements.

use jiff::civil::date;
use lb_core::Money;
use lb_feeds::ofx;
use lb_feeds::{AccountKind, FileKind, TrnType};
use pretty_assertions::assert_eq;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/testdata/ofx/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn ofx_1_sgml_checking_with_1252_and_crlf() {
    let bytes = fixture("checking-sgml-102.ofx");
    assert!(bytes.windows(2).any(|w| w == b"\r\n"), "fixture is CRLF");
    assert_eq!(lb_feeds::sniff(&bytes), FileKind::Ofx);
    let doc = ofx::parse(&bytes).unwrap();
    assert_eq!(doc.version, Some(102));
    assert_eq!(doc.header.get("CHARSET").map(String::as_str), Some("1252"));
    assert!(doc.signon.as_ref().unwrap().is_ok());
    let [s] = doc.statements.as_slice() else {
        panic!("one statement expected")
    };
    assert_eq!(s.account.bank_id.as_deref(), Some("999999999"));
    assert_eq!(s.account.account_id_masked, "****5678");
    assert_ne!(s.account.account_key, "");
    assert_eq!(s.account.account_type, AccountKind::Checking);
    assert_eq!(s.account.currency.as_deref(), Some("USD"));
    assert_eq!(
        s.account.institution.as_deref(),
        Some("Example Community Bank")
    );
    assert_eq!(s.account.fid.as_deref(), Some("99901"));
    assert_eq!(s.start, Some(date(2026, 10, 1)));
    assert_eq!(s.end, Some(date(2026, 10, 14)));
    assert_eq!(s.ledger_balance, Some(Money(519_851)));
    assert_eq!(s.available_balance, Some(Money(509_851)));
    assert_eq!(s.balance_date, Some(date(2026, 10, 14)));
    assert_eq!(s.warnings, Vec::<String>::new());

    let t = &s.transactions;
    assert_eq!(t.len(), 5);
    assert_eq!(t[0].fitid, "202610010001");
    assert_eq!(t[0].amount, Money(250_000));
    assert_eq!(t[0].trntype, TrnType::DirectDeposit);
    assert_eq!(t[0].payee.as_deref(), Some("ACME PAYROLL"));
    assert_eq!(t[0].memo.as_deref(), Some("DIRECT DEP PPD ID: 9999999999"));
    // [-8:PST] doesn't move the date.
    assert_eq!(t[0].posted, date(2026, 10, 1));
    assert_eq!(t[1].check_number.as_deref(), Some("1041"));
    assert_eq!(t[1].trntype, TrnType::Check);
    // Windows-1252 É.
    assert_eq!(t[2].payee.as_deref(), Some("SQ *CAFÉ BLEU #0412"));
    // Midnight GMT is still the 7th.
    assert_eq!(t[3].posted, date(2026, 10, 7));
    assert_eq!(t[4].trntype, TrnType::ServiceCharge);
    assert_eq!(t[4].raw["NAME"], "MONTHLY SERVICE FEE");
    assert!(t.iter().all(|x| !x.fitid_synthesized));
}

#[test]
fn ofx_2_xml_credit_card_qfx_with_bom() {
    let bytes = fixture("card-xml-220.qfx");
    assert!(bytes.starts_with(b"\xEF\xBB\xBF"));
    let doc = ofx::parse(&bytes).unwrap();
    assert_eq!(doc.version, Some(220));
    let s = &doc.statements[0];
    assert_eq!(s.account.account_type, AccountKind::CreditCard);
    assert_eq!(s.account.account_id_masked, "****7890");
    assert_eq!(s.account.bank_id, None);
    assert_eq!(s.account.intu_bid.as_deref(), Some("77701"));
    assert_eq!(s.ledger_balance, Some(Money(-77_218)));
    let t = &s.transactions;
    assert_eq!(t.len(), 4);
    assert_eq!(t[0].amount, Money(-4217));
    assert_eq!(t[0].posted, date(2026, 9, 20), "DTPOSTED wins over DTUSER");
    assert_eq!(t[0].payee.as_deref(), Some("AMZN Mktp US*2K4AB1CD2"));
    assert_eq!(t[1].amount, Money(-125_000), "1,250.00 with a comma");
    assert_eq!(t[1].memo, None, "empty MEMO");
    assert_eq!(t[2].amount, Money(50_000));
    // PAYEE aggregate, UTF-8 accents.
    assert_eq!(t[3].payee.as_deref(), Some("Café Résumé Books"));
    assert_eq!(t[3].raw["PAYEE"]["CITY"], "Springfield");
}

#[test]
fn web_connect_qbo_on_one_line_with_problems() {
    let s = ofx::parse_statements(&fixture("webconnect.qbo"))
        .unwrap()
        .remove(0);
    assert_eq!(s.account.intu_bid.as_deref(), Some("3000"));
    assert_eq!(s.account.institution.as_deref(), Some("B1"));
    assert_eq!(s.account.account_type, AccountKind::Savings);
    assert_eq!(s.ledger_balance, Some(Money(1_088_267)));
    let ids: Vec<&str> = s.transactions.iter().map(|t| t.fitid.as_str()).collect();
    assert_eq!(ids.len(), 6, "{ids:?}");
    assert_eq!(ids[0], "A1");
    assert_eq!(s.transactions[0].amount, Money(100_000), "+1,000.00");
    assert_eq!(s.transactions[1].amount, Money(42), "0.415 rounds half up");
    // Two AT&T payments with no FITID get distinct, stable ids.
    assert!(ids[2].starts_with("lb1:") && ids[2].ends_with(":0"));
    assert!(ids[3].starts_with("lb1:") && ids[3].ends_with(":1"));
    assert_eq!(s.transactions[2].payee.as_deref(), Some("AT&T MOBILITY"));
    // A5 listed twice is kept once; A5 reused for another transaction is renamed.
    assert_eq!(ids[4], "A5");
    assert_eq!(ids[5], "A5#2");
    assert_eq!(s.transactions[5].payee.as_deref(), Some("PARKING METER"));
    // The undated and unreadable rows are reported.
    assert_eq!(s.warnings.len(), 4, "{:?}", s.warnings);
    assert!(s.warnings.iter().any(|w| w.contains("has no date")));
    assert!(s.warnings.iter().any(|w| w.contains("\"lots\"")));
    // Same file again: same ids.
    let again = ofx::parse_statements(&fixture("webconnect.qbo"))
        .unwrap()
        .remove(0);
    assert_eq!(again.transactions, s.transactions);
}

#[test]
fn several_accounts_errors_and_investments_in_one_file() {
    let doc = ofx::parse(&fixture("multi-account.ofx")).unwrap();
    assert_eq!(doc.version, Some(103));
    let kinds: Vec<AccountKind> = doc
        .statements
        .iter()
        .map(|s| s.account.account_type)
        .collect();
    assert_eq!(
        kinds,
        vec![
            AccountKind::Checking,
            AccountKind::MoneyMarket,
            AccountKind::CreditCard
        ]
    );
    assert_ne!(
        doc.statements[0].account.account_key,
        doc.statements[1].account.account_key
    );
    assert_eq!(doc.statement_errors.len(), 1);
    assert_eq!(doc.statement_errors[0].code, 2003);
    assert!(
        doc.statement_errors[0]
            .describe()
            .contains("Account 9999 is closed")
    );
    assert_eq!(
        doc.warnings,
        vec!["investment statements in this file were skipped"]
    );
    // A correction is flagged for a person.
    assert!(
        doc.statements[1]
            .warnings
            .iter()
            .any(|w| w.contains("replace an earlier one (S-0)"))
    );
}

#[test]
fn a_signon_error_is_the_banks_message() {
    let err = ofx::parse_statements(&fixture("signon-error.ofx")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "the bank: the bank rejected the user name or password (code 15500): Invalid user ID or password"
    );
}

#[test]
fn lower_case_tags_tabs_and_odd_numbers() {
    let s = ofx::parse_statements(&fixture("messy-lowercase.ofx"))
        .unwrap()
        .remove(0);
    assert_eq!(s.account.bank_id.as_deref(), Some("666666666"));
    assert_eq!(s.account.account_id_masked, "****4321");
    let t = &s.transactions;
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].fitid, "X1");
    assert_eq!(t[0].amount, Money(-15_000));
    assert_eq!(t[0].check_number.as_deref(), Some("1042"), "from REFNUM");
    assert_eq!(
        t[0].payee.as_deref(),
        Some("CHECK #1042"),
        "memo-only payee"
    );
    assert_eq!(t[1].posted, date(2026, 10, 9), "ISO date");
    assert_eq!(t[1].amount, Money(150_000), "$ 1,500.00");
    assert_eq!(t[2].amount, Money(-345), "decimal comma");
    assert_eq!(
        s.start,
        Some(date(2026, 10, 8)),
        "period from the transactions"
    );
}

#[test]
fn not_ofx_at_all() {
    let csv = b"Date,Description,Amount\n10/01/2026,x,1.00\n";
    assert_eq!(lb_feeds::sniff(csv), FileKind::Csv);
    assert!(ofx::parse(csv).is_err());
    let empty = b"OFXHEADER:100\r\n\r\n<OFX></OFX>";
    let err = ofx::parse_statements(empty).unwrap_err();
    assert!(
        err.to_string()
            .contains("no bank or credit card statements")
    );
}
