//! The Web Connector transport's half: a qbXML response turns into exactly the
//! JSON the bridge serves, so the same normaliser and sync engine take it.

use lb_qbd::QbdEntity;
use lb_qbd::qbxml::xml::parse_response;
use lb_qbd::sync::SyncOptions;
use pretty_assertions::assert_eq;

use crate::common::*;

#[tokio::test]
async fn qbxml_converts_to_the_bridges_json_and_imports_the_same() {
    let xml = std::fs::read_to_string(testdata().join("qbxml/check_query_rs.xml")).unwrap();
    let rs = parse_response(&xml).unwrap();
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].name, "CheckQueryRs");
    assert_eq!(rs[0].status_code, 0);
    assert_eq!(rs[0].iterator_remaining_count, Some(0));
    let (element, from_xml) = &rs[0].records[0];
    assert_eq!(element, QbdEntity::Check.ret_element());

    // Byte-for-byte the same shape as the bridge's JSON for the same check.
    let src = larkspur();
    let from_bridge = src
        .records(QbdEntity::Check)
        .into_iter()
        .find(|c| c["TxnID"] == "5010-1700006100")
        .unwrap();
    assert_eq!(from_xml, &from_bridge);

    // And it imports to the same books: swap the bridge's record for the XML one.
    let b1 = books();
    run_clean(&b1, &src, &SyncOptions::default()).await;
    let via_xml = larkspur();
    via_xml.put(QbdEntity::Check, from_xml.clone());
    let b2 = books();
    run_clean(&b2, &via_xml, &SyncOptions::default()).await;
    let a = txn(&b1, QbdEntity::Check, "5010-1700006100");
    let b = txn(&b2, QbdEntity::Check, "5010-1700006100");
    assert_eq!(a.lines.len(), b.lines.len());
    assert_eq!(a.amount, b.amount);
}
