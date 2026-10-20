//! Passkey enrolment and sign-in, end to end, with a software authenticator.

mod common;

use common::*;
use lb_auth::{
    AuthError, AuthStore, EnrolFinish, EnrolmentPurpose, LoginFinish, MintedEnrolment,
    NewEnrolment, Principal, Role, SessionKind, User,
};

/// Bootstrap and enrol the first admin with `auth` at `origin`.
fn first_admin(store: &AuthStore, auth: &mut Authenticator) -> (User, Principal) {
    let token = store.bootstrap().unwrap().unwrap();
    enrol(store, &token, auth, Some("Ada Lovelace"))
}

fn enrol(
    store: &AuthStore,
    token: &MintedEnrolment,
    auth: &mut Authenticator,
    name: Option<&str>,
) -> (User, Principal) {
    let start = store
        .start_enrolment(&begin(&token.token), ORIGIN, &client())
        .unwrap();
    let credential = auth.register(&start.options, &Client::at(ORIGIN));
    let (user, session) = store
        .finish_enrolment(
            &EnrolFinish {
                token: token.token.clone(),
                challenge_id: start.challenge_id,
                credential,
                display_name: name.map(str::to_owned),
                passkey_name: None,
            },
            ORIGIN,
            &client(),
        )
        .unwrap();
    let principal = store.validate_session(&session.token).unwrap();
    (user, principal)
}

fn login(store: &AuthStore, auth: &mut Authenticator, c: &Client) -> Result<Principal, AuthError> {
    let start = store.start_login(ORIGIN, &client())?;
    let credential = auth.login(&start.options, c);
    let (_, session) = store.finish_login(
        &LoginFinish {
            challenge_id: start.challenge_id,
            credential,
        },
        ORIGIN,
        &client(),
    )?;
    store.validate_session(&session.token)
}

#[test]
fn first_admin_enrols_and_signs_in() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    assert_eq!(token.purpose, EnrolmentPurpose::FirstAdmin);
    assert!(token.token.starts_with("lbe_"));

    let start = store
        .start_enrolment(&begin(&token.token), ORIGIN, &client())
        .unwrap();
    assert_eq!(start.purpose, EnrolmentPurpose::FirstAdmin);
    // The options are PublicKeyCredentialCreationOptionsJSON, as
    // @simplewebauthn/browser's startRegistration({ optionsJSON }) takes them.
    let o = &start.options;
    assert_eq!(o["rp"]["id"], RP_ID);
    assert_eq!(o["rp"]["name"], "LibreBooks");
    assert_eq!(unb64(o["user"]["id"].as_str().unwrap()).len(), 16);
    assert!(o["user"]["name"].is_string());
    assert!(o["user"]["displayName"].is_string());
    assert_eq!(unb64(o["challenge"].as_str().unwrap()).len(), 16);
    assert_eq!(o["authenticatorSelection"]["userVerification"], "required");
    assert_eq!(o["authenticatorSelection"]["residentKey"], "required");
    assert_eq!(o["attestation"], "none");
    assert_eq!(o["timeout"], 300_000);
    let algs: Vec<i64> = o["pubKeyCredParams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["alg"].as_i64().unwrap())
        .collect();
    assert!(algs.contains(&-7) && algs.contains(&-8));

    let mut auth = Authenticator::new();
    let credential = auth.register(&start.options, &Client::at(ORIGIN));
    let (user, session) = store
        .finish_enrolment(
            &EnrolFinish {
                token: token.token.clone(),
                challenge_id: start.challenge_id,
                credential,
                display_name: Some("Ada Lovelace".to_owned()),
                passkey_name: None,
            },
            ORIGIN,
            &client(),
        )
        .unwrap();
    assert_eq!(user.role, Role::Admin);
    assert_eq!(user.display_name, "Ada Lovelace");
    // The WebAuthn user handle is the user's id.
    assert_eq!(
        auth.user_handle.as_deref().unwrap(),
        user.id.0.as_bytes().as_slice()
    );
    assert_eq!(session.info.kind, SessionKind::Passkey);
    assert!(session.token.starts_with("lbs_"));

    let me = store.validate_session(&session.token).unwrap();
    assert_eq!(me.user_id, user.id);
    assert_eq!(me.role, Role::Admin);

    let passkeys = store.list_passkeys(user.id).unwrap();
    assert_eq!(passkeys.len(), 1);
    assert_eq!(passkeys[0].name, "Chrome on Windows");
    assert_eq!(passkeys[0].rp_id, RP_ID);
    assert!(passkeys[0].backup_eligible && passkeys[0].backed_up);

    // The token is spent, and no new first-admin token is minted.
    let again = store.start_enrolment(&begin(&token.token), ORIGIN, &client());
    assert!(matches!(again, Err(AuthError::TokenUsed)), "{again:?}");
    assert!(store.bootstrap().unwrap().is_none());

    // Usernameless sign-in with the same passkey.
    let start = store.start_login(ORIGIN, &client()).unwrap();
    assert_eq!(start.options["rpId"], RP_ID);
    assert_eq!(start.options["userVerification"], "required");
    assert_eq!(start.options["allowCredentials"], serde_json::json!([]));
    let me = login(&store, &mut auth, &Client::at(ORIGIN)).unwrap();
    assert_eq!(me.user_id, user.id);
    assert_eq!(me.kind, SessionKind::Passkey);
    assert!(
        store.list_passkeys(user.id).unwrap()[0]
            .last_used_at
            .is_some()
    );
}

#[test]
fn zero_counter_passkeys_sign_in_repeatedly() {
    let (store, _clock) = store();
    let mut auth = Authenticator::new();
    assert!(auth.counter.is_none());
    first_admin(&store, &mut auth);
    for _ in 0..3 {
        login(&store, &mut auth, &Client::at(ORIGIN)).unwrap();
    }
}

#[test]
fn counting_authenticator_rejects_a_counter_that_goes_backwards() {
    let (store, _clock) = store();
    let mut auth = Authenticator::counting();
    first_admin(&store, &mut auth);
    login(&store, &mut auth, &Client::at(ORIGIN)).unwrap();
    login(&store, &mut auth, &Client::at(ORIGIN)).unwrap();
    // A clone of the key that's behind on its counter.
    let mut clone = auth.clone();
    clone.counter = Some(0);
    let err = login(&store, &mut clone, &Client::at(ORIGIN)).unwrap_err();
    assert_verification_failed(&err, "counter");
    // The original keeps working.
    login(&store, &mut auth, &Client::at(ORIGIN)).unwrap();
}

#[test]
fn registration_rejects_a_wrong_origin_rp_id_challenge_type_or_missing_uv() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    let cases: Vec<(&str, Client)> = vec![
        ("origin", Client::at("https://evil.example.net")),
        (
            "subdomain origin not configured",
            Client::at("https://x.books.example.com"),
        ),
        (
            "rp id",
            Client {
                rp_id: Some("evil.example.net".to_owned()),
                ..Client::at(ORIGIN)
            },
        ),
        (
            "challenge",
            Client {
                challenge: Some(b64(&[7u8; 16])),
                ..Client::at(ORIGIN)
            },
        ),
        (
            "type",
            Client {
                typ: Some("webauthn.get"),
                ..Client::at(ORIGIN)
            },
        ),
        (
            "user verification",
            Client {
                uv: false,
                ..Client::at(ORIGIN)
            },
        ),
    ];
    for (what, c) in cases {
        let start = store
            .start_enrolment(&begin(&token.token), ORIGIN, &client())
            .unwrap();
        let credential = Authenticator::new().register(&start.options, &c);
        let err = store
            .finish_enrolment(
                &EnrolFinish {
                    token: token.token.clone(),
                    challenge_id: start.challenge_id,
                    credential,
                    display_name: None,
                    passkey_name: None,
                },
                ORIGIN,
                &client(),
            )
            .unwrap_err();
        assert_verification_failed(&err, what);
    }
    // None of those spent the token.
    first_admin_with(&store, &token);
}

/// The response was read fine and then failed a WebAuthn check (not, say, a
/// parse error that would make the test pass for the wrong reason).
fn assert_verification_failed(err: &AuthError, what: &str) {
    match err {
        AuthError::Passkey(msg) => {
            assert!(!msg.contains("unreadable"), "{what}: {msg}");
            eprintln!("{what}: {msg}");
        }
        other => panic!("{what}: expected a verification failure, got {other:?}"),
    }
}

fn first_admin_with(store: &AuthStore, token: &MintedEnrolment) {
    let mut auth = Authenticator::new();
    enrol(store, token, &mut auth, None);
}

#[test]
fn sign_in_rejects_a_wrong_origin_rp_id_challenge_or_missing_uv() {
    let (store, _clock) = store();
    let mut auth = Authenticator::new();
    first_admin(&store, &mut auth);
    let cases: Vec<(&str, Client)> = vec![
        ("origin", Client::at("https://evil.example.net")),
        (
            "rp id",
            Client {
                rp_id: Some("evil.example.net".to_owned()),
                ..Client::at(ORIGIN)
            },
        ),
        (
            "challenge",
            Client {
                challenge: Some(b64(&[7u8; 16])),
                ..Client::at(ORIGIN)
            },
        ),
        (
            "type",
            Client {
                typ: Some("webauthn.create"),
                ..Client::at(ORIGIN)
            },
        ),
        (
            "user verification",
            Client {
                uv: false,
                ..Client::at(ORIGIN)
            },
        ),
    ];
    for (what, c) in cases {
        let err = login(&store, &mut auth, &c).unwrap_err();
        assert_verification_failed(&err, what);
    }
    login(&store, &mut auth, &Client::at(ORIGIN)).unwrap();
}

#[test]
fn a_ceremony_is_used_once_and_expires() {
    let (store, clock) = store();
    let mut auth = Authenticator::new();
    first_admin(&store, &mut auth);

    // Replaying a finished sign-in.
    let start = store.start_login(ORIGIN, &client()).unwrap();
    let credential = auth.login(&start.options, &Client::at(ORIGIN));
    let finish = LoginFinish {
        challenge_id: start.challenge_id,
        credential,
    };
    store.finish_login(&finish, ORIGIN, &client()).unwrap();
    let err = store.finish_login(&finish, ORIGIN, &client()).unwrap_err();
    assert!(matches!(err, AuthError::ChallengeNotFound), "{err:?}");

    // Finishing too late.
    let start = store.start_login(ORIGIN, &client()).unwrap();
    let credential = auth.login(&start.options, &Client::at(ORIGIN));
    clock.advance(minutes(6));
    let err = store
        .finish_login(
            &LoginFinish {
                challenge_id: start.challenge_id,
                credential,
            },
            ORIGIN,
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::ChallengeNotFound), "{err:?}");
}

#[test]
fn origins_pick_the_relying_party() {
    let (store, _clock) = store();
    let mut auth = Authenticator::new();
    first_admin(&store, &mut auth);

    let err = store
        .start_login("https://evil.example.net", &client())
        .unwrap_err();
    assert!(matches!(err, AuthError::UnknownOrigin { .. }), "{err:?}");

    // The local RP is configured, so sign-in there starts, with its own RP ID…
    let start = store.start_login(LOCAL_ORIGIN, &client()).unwrap();
    assert_eq!(start.options["rpId"], "localhost");
    // …but a passkey bound to books.example.com is not one of its passkeys.
    let credential = auth.login(&start.options, &Client::at(LOCAL_ORIGIN));
    let err = store
        .finish_login(
            &LoginFinish {
                challenge_id: start.challenge_id,
                credential,
            },
            LOCAL_ORIGIN,
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::UnknownCredential), "{err:?}");

    // A ceremony started at one origin can't be finished at another.
    let start = store.start_login(ORIGIN, &client()).unwrap();
    let credential = auth.login(&start.options, &Client::at(ORIGIN));
    let err = store
        .finish_login(
            &LoginFinish {
                challenge_id: start.challenge_id,
                credential,
            },
            LOCAL_ORIGIN,
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::UnknownOrigin { .. }), "{err:?}");
}

#[test]
fn a_removed_passkey_can_no_longer_sign_in_and_its_sessions_end() {
    let (store, _clock) = store();
    let mut first = Authenticator::new();
    let (admin, me) = first_admin(&store, &mut first);

    // Add a second passkey with an add_device token minted for myself.
    let token = store
        .create_enrolment(
            &me,
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
    let start = store
        .start_enrolment(&begin(&token.token), ORIGIN, &client())
        .unwrap();
    assert_eq!(start.purpose, EnrolmentPurpose::AddDevice);
    assert_eq!(start.display_name.as_deref(), Some("Ada Lovelace"));
    // The existing passkey is excluded, so the authenticator won't make a twin.
    let excluded = start.options["excludeCredentials"].as_array().unwrap();
    assert_eq!(excluded.len(), 1);
    assert_eq!(excluded[0]["id"], b64(&first.credential_id));
    // The user handle is the existing user's.
    assert_eq!(
        unb64(start.options["user"]["id"].as_str().unwrap()),
        admin.id.0.as_bytes()
    );
    let mut second = Authenticator::new();
    let credential = second.register(&start.options, &Client::at(ORIGIN));
    let (user, _) = store
        .finish_enrolment(
            &EnrolFinish {
                token: token.token.clone(),
                challenge_id: start.challenge_id,
                credential,
                display_name: Some("ignored for add_device".to_owned()),
                passkey_name: Some("Phone".to_owned()),
            },
            ORIGIN,
            &client(),
        )
        .unwrap();
    assert_eq!(user.id, admin.id);
    assert_eq!(user.display_name, "Ada Lovelace");
    let passkeys = store.list_passkeys(admin.id).unwrap();
    assert_eq!(passkeys.len(), 2);
    assert_eq!(passkeys[0].name, "Phone");

    // Sign in with the second, then remove it: its session ends, and it can't
    // sign in again.
    let second_session = login(&store, &mut second, &Client::at(ORIGIN)).unwrap();
    let phone = passkeys[0].id;
    store.delete_passkey(&me, phone).unwrap();
    assert!(
        store
            .list_sessions(admin.id, None)
            .unwrap()
            .iter()
            .all(|s| s.id != second_session.session_id)
    );
    let err = login(&store, &mut second, &Client::at(ORIGIN)).unwrap_err();
    assert!(matches!(err, AuthError::UnknownCredential), "{err:?}");

    // The last passkey can't be removed by its owner.
    let last = store.list_passkeys(admin.id).unwrap()[0].id;
    let err = store.delete_passkey(&me, last).unwrap_err();
    assert!(matches!(err, AuthError::Conflict(_)), "{err:?}");
}

#[test]
fn new_user_tokens_need_a_name_and_give_the_tokens_role_and_company() {
    let (store, _clock) = store();
    let (_, admin) = first_admin(&store, &mut Authenticator::new());
    let company = lb_auth::CompanyId::new();
    let token = store
        .create_enrolment(
            &admin,
            &NewEnrolment {
                purpose: EnrolmentPurpose::NewUser,
                role: Some(Role::Viewer),
                user_id: None,
                company_id: Some(company),
                display_name: None,
                ttl_minutes: Some(30),
            },
        )
        .unwrap();
    let start = store
        .start_enrolment(&begin(&token.token), ORIGIN, &client())
        .unwrap();
    assert_eq!(start.purpose, EnrolmentPurpose::NewUser);
    assert_eq!(start.display_name, None);
    let credential = Authenticator::new().register(&start.options, &Client::at(ORIGIN));
    let mut finish = EnrolFinish {
        token: token.token.clone(),
        challenge_id: start.challenge_id,
        credential,
        display_name: None,
        passkey_name: None,
    };
    let err = store
        .finish_enrolment(&finish, ORIGIN, &client())
        .unwrap_err();
    assert!(matches!(err, AuthError::Invalid(_)), "{err:?}");

    // The ceremony was kept: the page asks for the name and resubmits the same
    // response, with no second authenticator prompt.
    finish.display_name = Some("Grace Hopper".to_owned());
    let (user, session) = store.finish_enrolment(&finish, ORIGIN, &client()).unwrap();
    let viewer = store.validate_session(&session.token).unwrap();
    assert_eq!(user.display_name, "Grace Hopper");
    assert_eq!(user.role, Role::Viewer);
    assert_eq!(viewer.role, Role::Viewer);
    assert_eq!(
        store.company_role(&viewer, company).unwrap(),
        Some(Role::Viewer)
    );
}

#[test]
fn a_name_given_at_the_start_labels_the_passkey_and_names_the_user() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    let start = store
        .start_enrolment(
            &lb_auth::EnrolStartRequest {
                token: token.token.clone(),
                display_name: Some(" Ada Lovelace ".to_owned()),
            },
            ORIGIN,
            &client(),
        )
        .unwrap();
    assert_eq!(start.display_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(start.options["user"]["displayName"], "Ada Lovelace");
    assert_eq!(start.options["user"]["name"], "Ada.Lovelace");
    let credential = Authenticator::new().register(&start.options, &Client::at(ORIGIN));
    let (user, _) = store
        .finish_enrolment(
            &EnrolFinish {
                token: token.token.clone(),
                challenge_id: start.challenge_id,
                credential,
                display_name: None,
                passkey_name: None,
            },
            ORIGIN,
            &client(),
        )
        .unwrap();
    assert_eq!(user.display_name, "Ada Lovelace");
}

#[test]
fn finishing_after_the_token_expired_fails() {
    let (store, clock) = store();
    let (_, admin) = first_admin(&store, &mut Authenticator::new());
    let token = store
        .create_enrolment(
            &admin,
            &NewEnrolment {
                purpose: EnrolmentPurpose::NewUser,
                role: None,
                user_id: None,
                company_id: None,
                display_name: Some("Late Larry".to_owned()),
                ttl_minutes: None,
            },
        )
        .unwrap();
    // Start just inside the 15 minutes, finish just outside.
    clock.advance(minutes(14));
    let start = store
        .start_enrolment(&begin(&token.token), ORIGIN, &client())
        .unwrap();
    let credential = Authenticator::new().register(&start.options, &Client::at(ORIGIN));
    clock.advance(minutes(2));
    let err = store
        .finish_enrolment(
            &EnrolFinish {
                token: token.token.clone(),
                challenge_id: start.challenge_id,
                credential,
                display_name: None,
                passkey_name: None,
            },
            ORIGIN,
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::TokenExpired), "{err:?}");
}

#[test]
fn finishing_with_a_different_token_fails() {
    let (store, _clock) = store();
    let (_, admin) = first_admin(&store, &mut Authenticator::new());
    let new_user = NewEnrolment {
        purpose: EnrolmentPurpose::NewUser,
        role: None,
        user_id: None,
        company_id: None,
        display_name: Some("Someone".to_owned()),
        ttl_minutes: None,
    };
    let a = store.create_enrolment(&admin, &new_user).unwrap();
    let b = store.create_enrolment(&admin, &new_user).unwrap();
    let start = store
        .start_enrolment(&begin(&a.token), ORIGIN, &client())
        .unwrap();
    let credential = Authenticator::new().register(&start.options, &Client::at(ORIGIN));
    let err = store
        .finish_enrolment(
            &EnrolFinish {
                token: b.token.clone(),
                challenge_id: start.challenge_id,
                credential,
                display_name: None,
                passkey_name: None,
            },
            ORIGIN,
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::TokenNotFound), "{err:?}");
}

#[test]
fn two_racing_enrolments_with_one_token_make_one_user() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    let finishes: Vec<EnrolFinish> = (0..2)
        .map(|_| {
            let start = store
                .start_enrolment(&begin(&token.token), ORIGIN, &client())
                .unwrap();
            EnrolFinish {
                token: token.token.clone(),
                challenge_id: start.challenge_id,
                credential: Authenticator::new().register(&start.options, &Client::at(ORIGIN)),
                display_name: Some("Racer".to_owned()),
                passkey_name: None,
            }
        })
        .collect();
    let barrier = std::sync::Barrier::new(2);
    let results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = finishes
            .iter()
            .map(|f| {
                let store = store.clone();
                let barrier = &barrier;
                s.spawn(move || {
                    barrier.wait();
                    store.finish_enrolment(f, ORIGIN, &client())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let ok = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(ok, 1, "{results:?}");
    assert!(
        results
            .iter()
            .any(|r| matches!(r, Err(AuthError::TokenUsed))),
        "{results:?}"
    );
    assert_eq!(store.list_users().unwrap().len(), 1);
}
