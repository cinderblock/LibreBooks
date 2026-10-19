//! Enrolment tokens, bootstrap, users, permissions, rate limits, config.

mod common;

use common::*;
use ed25519_dalek::SigningKey;
use lb_auth::{
    AuthConfig, AuthError, AuthStore, CompanyId, DeviceEnrol, EnrolmentPurpose, NewEnrolment,
    Permission, Principal, RelyingParty, Role, User, UserUpdate,
};

fn new_user(role: Option<Role>) -> NewEnrolment {
    NewEnrolment {
        purpose: EnrolmentPurpose::NewUser,
        role,
        user_id: None,
        company_id: None,
        display_name: Some("New Person".to_owned()),
        ttl_minutes: None,
    }
}

/// Enrol a device with `token` and sign it in.
fn join(store: &AuthStore, token: &str) -> (User, Principal) {
    let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let (user, info) = store
        .enrol_device(
            &DeviceEnrol {
                token: token.to_owned(),
                public_key: b64(key.verifying_key().as_bytes()),
                name: "Laptop".to_owned(),
                display_name: None,
            },
            &client(),
        )
        .unwrap();
    let challenge = store.device_challenge(&client()).unwrap();
    let bytes: [u8; 32] = unb64(&challenge.challenge).try_into().unwrap();
    use ed25519_dalek::Signer as _;
    let (_, session) = store
        .device_login(
            &lb_auth::DeviceLogin {
                challenge_id: challenge.challenge_id,
                key_id: info.id,
                signature: b64(&key
                    .sign(&lb_auth::device_login_message(&bytes, ORIGIN))
                    .to_bytes()),
            },
            ORIGIN,
            &client(),
        )
        .unwrap();
    (user, store.validate_session(&session.token).unwrap())
}

fn admin(store: &AuthStore) -> (User, Principal) {
    let token = store.bootstrap().unwrap().unwrap();
    join(store, &token.token)
}

#[test]
fn bootstrap_mints_one_live_first_admin_token_until_an_admin_exists() {
    let (store, clock) = store();
    assert!(!store.has_users().unwrap());
    let first = store.bootstrap().unwrap().unwrap();
    assert_eq!(first.purpose, EnrolmentPurpose::FirstAdmin);
    assert_eq!(
        first.expires_at,
        start_time() + jiff::SignedDuration::from_hours(1)
    );
    // A restart prints a new one and the old one stops working.
    let second = store.bootstrap().unwrap().unwrap();
    assert_ne!(first.token, second.token);
    let err = store
        .inspect_enrolment(&first.token, &client())
        .unwrap_err();
    assert!(matches!(err, AuthError::TokenRevoked), "{err:?}");
    let info = store.inspect_enrolment(&second.token, &client()).unwrap();
    assert_eq!(info.purpose, EnrolmentPurpose::FirstAdmin);
    assert_eq!(store.list_enrolments().unwrap().len(), 1);

    // It lasts an hour.
    clock.advance(minutes(61));
    let err = store
        .inspect_enrolment(&second.token, &client())
        .unwrap_err();
    assert!(matches!(err, AuthError::TokenExpired), "{err:?}");

    let third = store.bootstrap().unwrap().unwrap();
    let (user, _) = join(&store, &third.token);
    assert_eq!(user.role, Role::Admin);
    assert!(store.bootstrap().unwrap().is_none());
    assert!(store.has_users().unwrap());
}

#[test]
fn a_first_admin_token_is_dead_once_another_admin_exists() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    // The operator makes an admin by hand before the token is used.
    store.create_user("Operator", Role::Admin).unwrap();
    let err = store
        .start_enrolment(&begin(&token.token), ORIGIN, &client())
        .unwrap_err();
    assert!(matches!(err, AuthError::AdminExists), "{err:?}");
    let err = store
        .enrol_device(
            &DeviceEnrol {
                token: token.token.clone(),
                public_key: b64(SigningKey::from_bytes(&[9; 32]).verifying_key().as_bytes()),
                name: "x".to_owned(),
                display_name: None,
            },
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::AdminExists), "{err:?}");
}

#[test]
fn tokens_expire_and_respect_requested_lifetimes() {
    let (store, clock) = store();
    let (_, me) = admin(&store);
    let default = store.create_enrolment(&me, &new_user(None)).unwrap();
    let long = store
        .create_enrolment(
            &me,
            &NewEnrolment {
                ttl_minutes: Some(60),
                ..new_user(None)
            },
        )
        .unwrap();
    assert_eq!(
        default.expires_at,
        start_time() + jiff::SignedDuration::from_mins(15)
    );
    clock.advance(minutes(15));
    let err = store
        .inspect_enrolment(&default.token, &client())
        .unwrap_err();
    assert!(matches!(err, AuthError::TokenExpired), "{err:?}");
    store.inspect_enrolment(&long.token, &client()).unwrap();
    assert_eq!(store.list_enrolments().unwrap().len(), 1);

    for bad in [Some(0), Some(7 * 24 * 60 + 1)] {
        let err = store
            .create_enrolment(
                &me,
                &NewEnrolment {
                    ttl_minutes: bad,
                    ..new_user(None)
                },
            )
            .unwrap_err();
        assert!(matches!(err, AuthError::Invalid(_)), "{bad:?}: {err:?}");
    }
}

#[test]
fn only_hashes_of_tokens_are_stored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let clock = lb_auth::Clock::manual(start_time());
    let store = AuthStore::open(&path, config(&clock)).unwrap();
    let token = store.bootstrap().unwrap().unwrap();
    let (_, me) = join(&store, &token.token);
    let minted = store.create_enrolment(&me, &new_user(None)).unwrap();
    drop(store);
    let raw = std::fs::read(&path).unwrap();
    let wal = std::fs::read(dir.path().join("server.db-wal")).unwrap_or_default();
    for secret in [&token.token, &minted.token] {
        let body = secret.trim_start_matches("lbe_").as_bytes();
        let found = raw.windows(body.len()).any(|w| w == body)
            || wal.windows(body.len()).any(|w| w == body);
        assert!(!found, "a token was stored in clear");
    }
}

#[test]
fn who_may_mint_which_tokens() {
    let (store, _clock) = store();
    let (_, admin_me) = admin(&store);
    let bk_token = store
        .create_enrolment(&admin_me, &new_user(Some(Role::Bookkeeper)))
        .unwrap();
    let (bob, bob_me) = join(&store, &bk_token.token);
    assert_eq!(bob.role, Role::Bookkeeper);

    // A bookkeeper can't invite people…
    let err = store
        .create_enrolment(&bob_me, &new_user(None))
        .unwrap_err();
    assert!(matches!(err, AuthError::Forbidden(_)), "{err:?}");
    // …or add devices to someone else…
    let err = store
        .create_enrolment(
            &bob_me,
            &NewEnrolment {
                purpose: EnrolmentPurpose::AddDevice,
                role: None,
                user_id: Some(admin_me.user_id),
                company_id: None,
                display_name: None,
                ttl_minutes: None,
            },
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::Forbidden(_)), "{err:?}");
    // …but can add a device for himself.
    let mine = store
        .create_enrolment(
            &bob_me,
            &NewEnrolment {
                purpose: EnrolmentPurpose::AddDevice,
                role: None,
                user_id: None,
                company_id: None,
                display_name: None,
                ttl_minutes: None,
            },
        )
        .unwrap();
    let info = store.inspect_enrolment(&mine.token, &client()).unwrap();
    assert_eq!(info.user_id, Some(bob.id));
    assert_eq!(info.created_by, Some(bob.id));
    // He can cancel his own link; it's gone from the list.
    store.revoke_enrolment(&bob_me, mine.id).unwrap();
    let err = store.inspect_enrolment(&mine.token, &client()).unwrap_err();
    assert!(matches!(err, AuthError::TokenRevoked), "{err:?}");
    // First-admin tokens can't be minted on request.
    let err = store
        .create_enrolment(
            &admin_me,
            &NewEnrolment {
                purpose: EnrolmentPurpose::FirstAdmin,
                ..new_user(None)
            },
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::Invalid(_)), "{err:?}");
}

#[test]
fn roles_and_company_access() {
    let (store, _clock) = store();
    let (_, admin_me) = admin(&store);
    let books = CompanyId::new();
    let other = CompanyId::new();

    let (viewer, viewer_me) = join(
        &store,
        &store
            .create_enrolment(&admin_me, &new_user(Some(Role::Viewer)))
            .unwrap()
            .token,
    );
    let (bk, bk_me) = join(
        &store,
        &store
            .create_enrolment(&admin_me, &new_user(Some(Role::Bookkeeper)))
            .unwrap()
            .token,
    );

    // Admins can do anything anywhere.
    assert_eq!(
        store.company_role(&admin_me, other).unwrap(),
        Some(Role::Admin)
    );
    store
        .authorize_company(&admin_me, other, Permission::AdminBooks)
        .unwrap();
    admin_me.require(Permission::ManageServer).unwrap();

    // Others need a grant.
    assert_eq!(store.company_role(&bk_me, books).unwrap(), None);
    let err = store
        .authorize_company(&bk_me, books, Permission::ReadBooks)
        .unwrap_err();
    assert!(matches!(err, AuthError::Forbidden(_)), "{err:?}");
    store
        .grant_company_access(bk.id, books, Role::Admin)
        .unwrap();
    store
        .grant_company_access(viewer.id, books, Role::Bookkeeper)
        .unwrap();
    // A grant is capped by the server role.
    assert_eq!(
        store.company_role(&bk_me, books).unwrap(),
        Some(Role::Bookkeeper)
    );
    assert_eq!(
        store.company_role(&viewer_me, books).unwrap(),
        Some(Role::Viewer)
    );
    store
        .authorize_company(&bk_me, books, Permission::WriteBooks)
        .unwrap();
    assert!(
        store
            .authorize_company(&bk_me, books, Permission::AdminBooks)
            .is_err()
    );
    store
        .authorize_company(&viewer_me, books, Permission::ReadBooks)
        .unwrap();
    assert!(
        store
            .authorize_company(&viewer_me, books, Permission::WriteBooks)
            .is_err()
    );
    assert!(bk_me.require(Permission::ManageUsers).is_err());
    assert_eq!(store.company_access(bk.id).unwrap().len(), 1);
    store.revoke_company_access(bk.id, books).unwrap();
    assert_eq!(store.company_role(&bk_me, books).unwrap(), None);
}

#[test]
fn the_last_admin_is_protected() {
    let (store, _clock) = store();
    let (admin_user, admin_me) = admin(&store);
    for update in [
        UserUpdate {
            role: Some(Role::Viewer),
            ..UserUpdate::default()
        },
        UserUpdate {
            disabled: Some(true),
            ..UserUpdate::default()
        },
    ] {
        let err = store
            .update_user(&admin_me, admin_user.id, &update)
            .unwrap_err();
        assert!(matches!(err, AuthError::Conflict(_)), "{update:?}: {err:?}");
    }
    let err = store.delete_user(&admin_me, admin_user.id).unwrap_err();
    assert!(matches!(err, AuthError::Conflict(_)), "{err:?}");

    // With a second admin, the first may step down.
    let (second, second_me) = join(
        &store,
        &store
            .create_enrolment(&admin_me, &new_user(Some(Role::Admin)))
            .unwrap()
            .token,
    );
    let demoted = store
        .update_user(
            &admin_me,
            admin_user.id,
            &UserUpdate {
                role: Some(Role::Bookkeeper),
                ..UserUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(demoted.role, Role::Bookkeeper);
    // Roles are read per request, so the demotion is immediate.
    // (admin_me is stale; validate again to see it.)
    store.delete_user(&second_me, admin_user.id).unwrap();
    assert_eq!(
        store.list_users().unwrap(),
        vec![store.user(second.id).unwrap()]
    );
}

#[test]
fn people_may_rename_themselves_but_not_promote_themselves() {
    let (store, _clock) = store();
    let (_, admin_me) = admin(&store);
    let (bob, bob_me) = join(
        &store,
        &store
            .create_enrolment(&admin_me, &new_user(Some(Role::Viewer)))
            .unwrap()
            .token,
    );
    let renamed = store
        .update_user(
            &bob_me,
            bob.id,
            &UserUpdate {
                display_name: Some("  Robert  ".to_owned()),
                ..UserUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(renamed.display_name, "Robert");
    let err = store
        .update_user(
            &bob_me,
            bob.id,
            &UserUpdate {
                role: Some(Role::Admin),
                ..UserUpdate::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::Forbidden(_)), "{err:?}");
    let err = store
        .update_user(
            &bob_me,
            bob.id,
            &UserUpdate {
                display_name: Some("   ".to_owned()),
                ..UserUpdate::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::Invalid(_)), "{err:?}");
}

#[test]
fn failed_attempts_are_rate_limited_per_client() {
    let (store, clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    let attacker = client_at("198.51.100.66");
    for i in 0..20 {
        let err = store
            .inspect_enrolment(&format!("lbe_guess{i}"), &attacker)
            .unwrap_err();
        assert!(matches!(err, AuthError::TokenNotFound), "{err:?}");
    }
    // Now even a right guess is refused, and the error says how long to wait.
    let err = store
        .inspect_enrolment(&token.token, &attacker)
        .unwrap_err();
    assert!(matches!(err, AuthError::RateLimited { .. }), "{err:?}");
    assert_eq!(err.code(), "rate_limited");
    let err = store.start_login(ORIGIN, &attacker).unwrap_err();
    assert!(matches!(err, AuthError::RateLimited { .. }), "{err:?}");
    // Other clients are unaffected.
    store.inspect_enrolment(&token.token, &client()).unwrap();
    // The block lifts after the window.
    clock.advance(minutes(11));
    store.inspect_enrolment(&token.token, &attacker).unwrap();
}

#[test]
fn configuration_mistakes_are_caught_at_open() {
    let bad = [
        RelyingParty::new("192.168.1.10", "x", ["https://192.168.1.10"]),
        RelyingParty::new("books.example.com", "x", ["http://books.example.com"]),
        RelyingParty::new("books.example.com", "x", ["https://books.example.com/"]),
        RelyingParty::new("books.example.com", "x", ["https://books.example.com/app"]),
        RelyingParty::new("books.example.com", "x", ["https://other.example.com"]),
        RelyingParty::new("books.example.com", "x", Vec::<String>::new()),
        RelyingParty::new("books.example.com", "", ["https://books.example.com"]),
    ];
    for rp in bad {
        let config = AuthConfig {
            relying_parties: vec![rp.clone()],
            ..AuthConfig::default()
        };
        let err = AuthStore::in_memory(config).unwrap_err();
        assert!(matches!(err, AuthError::Config(_)), "{rp:?}: {err:?}");
    }
    // Subdomains and explicit ports are fine.
    AuthStore::in_memory(AuthConfig {
        relying_parties: vec![RelyingParty::new(
            "example.com",
            "LibreBooks",
            ["https://books.example.com", "https://example.com:8443"],
        )],
        ..AuthConfig::default()
    })
    .unwrap();
    // for_origin derives the RP ID from the host.
    let rp = RelyingParty::for_origin("https://books.example.com", "LibreBooks").unwrap();
    assert_eq!(rp.id, "books.example.com");
    assert_eq!(rp.origins, ["https://books.example.com"]);
    assert!(RelyingParty::for_origin("http://10.0.0.1:8740", "LibreBooks").is_err());
}

#[test]
fn errors_carry_api_codes() {
    assert_eq!(AuthError::Unauthenticated.code(), "unauthenticated");
    assert_eq!(AuthError::Forbidden("x".into()).code(), "forbidden");
    assert_eq!(AuthError::TokenExpired.code(), "invalid");
    assert_eq!(AuthError::AlreadyRegistered.code(), "conflict");
    assert_eq!(
        AuthError::NotFound {
            kind: "user",
            id: String::new()
        }
        .code(),
        "not_found"
    );
}
