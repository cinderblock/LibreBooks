//! Guards on `tauri.conf.json` that would otherwise only show up as a broken
//! window at run time.

/// Tauri's `freezePrototype` runs on every page, the lb-server SPA included,
/// and libraries that assign methods on their own prototypes (dayjs's
/// `valueOf`) then throw while the bundle starts: a white window. See
/// `plans/desktop.md`, Findings.
#[test]
fn object_prototype_is_not_frozen() {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
        .expect("tauri.conf.json parses");
    let freeze = conf
        .pointer("/app/security/freezePrototype")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    assert!(!freeze, "app.security.freezePrototype must stay false");
}
