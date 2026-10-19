//! Sessions: expiry (idle and absolute), revocation, disabled users.

mod common;

use std::time::Duration;

use common::*;
use ed25519_dalek::{Signer as _, SigningKey};
use lb_auth::{
    AuthError, AuthStore, DeviceEnrol, DeviceLogin, NewSession, Role, User, UserUpdate,
    device_login_message,
};

/// A user signed in with a device key (the quickest way to a real session).
fn signed_in(
    store: &AuthStore,
    token: &str,
) -> (User, SigningKey, lb_auth::DeviceKeyId, NewSession) {
    let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let (user, info) = store
        .enrol_device(
            &DeviceEnrol {
                token: token.to_owned(),
                public_key: b64(key.verifying_key().as_bytes()),
                name: "Laptop".to_owned(),
                display_name: Some("Someone".to_owned()),
            },
            &client(),
        )
        .unwrap();
    let session = sign_in(store, &key, info.id).unwrap();
    (user, key, info.id, session)
}

fn sign_in(
    store: &AuthStore,
    key: &SigningKey,
    key_id: lb_auth::DeviceKeyId,
) -> Result<NewSession, AuthError> {
    let challenge = store.device_challenge(&client())?;
    let bytes: [u8; 32] = unb64(&challenge.challenge).try_into().unwrap();
    store
        .device_login(
            &DeviceLogin {
                challenge_id: challenge.challenge_id,
                key_id,
                signature: b64(&key.sign(&device_login_message(&bytes, ORIGIN)).to_bytes()),
            },
            ORIGIN,
            &client(),
        )
        .map(|(_, s)| s)
}

fn admin(store: &AuthStore) -> (User, NewSession) {
    let token = store.bootstrap().unwrap().unwrap();
    let (user, _, _, session) = signed_in(store, &token.token);
    (user, session)
}

#[test]
fn an_unused_session_ends_after_the_idle_timeout() {
    let (store, clock) = store();
    let (_, session) = admin(&store);
    clock.advance(Duration::from_secs(14 * 24 * 60 * 60 - 1));
    store.validate_session(&session.token).unwrap();
    // That use reset the idle clock.
    clock.advance(days(13));
    store.validate_session(&session.token).unwrap();
    clock.advance(days(14) + Duration::from_secs(1));
    let err = store.validate_session(&session.token).unwrap_err();
    assert!(matches!(err, AuthError::Unauthenticated), "{err:?}");
}

#[test]
fn a_used_session_still_ends_at_its_absolute_expiry() {
    let (store, clock) = store();
    let (_, session) = admin(&store);
    for _ in 0..8 {
        clock.advance(days(10));
        store.validate_session(&session.token).unwrap();
    }
    // Day 89: still fine, and recently used.
    clock.advance(days(9));
    store.validate_session(&session.token).unwrap();
    // Day 90 and a second: over.
    clock.advance(days(1) + Duration::from_secs(1));
    let err = store.validate_session(&session.token).unwrap_err();
    assert!(matches!(err, AuthError::Unauthenticated), "{err:?}");
}

#[test]
fn last_seen_is_written_at_most_once_a_minute() {
    let (store, clock) = store();
    let (user, session) = admin(&store);
    let created = session.info.last_seen_at;
    clock.advance(Duration::from_secs(30));
    store.validate_session(&session.token).unwrap();
    assert_eq!(
        store.list_sessions(user.id, None).unwrap()[0].last_seen_at,
        created
    );
    clock.advance(Duration::from_secs(31));
    store.validate_session(&session.token).unwrap();
    let seen = store.list_sessions(user.id, None).unwrap()[0].last_seen_at;
    assert_eq!(seen, created + jiff::SignedDuration::from_secs(61));
}

#[test]
fn logout_and_revocation_end_sessions() {
    let (store, _clock) = store();
    let (user, first) = admin(&store);
    let me = store.validate_session(&first.token).unwrap();

    let sessions = store.list_sessions(user.id, Some(me.session_id)).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].current);

    store.logout(&first.token).unwrap();
    store.logout(&first.token).unwrap(); // twice is fine
    let err = store.validate_session(&first.token).unwrap_err();
    assert!(matches!(err, AuthError::Unauthenticated), "{err:?}");
    assert_eq!(store.list_sessions(user.id, None).unwrap(), []);

    // Garbage tokens are just "not signed in".
    for bad in [
        "",
        "lbs_",
        "lbe_whatever",
        "lbs_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        let err = store.validate_session(bad).unwrap_err();
        assert!(
            matches!(err, AuthError::Unauthenticated),
            "{bad:?}: {err:?}"
        );
    }
}

#[test]
fn people_revoke_their_own_sessions_and_admins_anyones() {
    let (store, _clock) = store();
    let (_, admin_session) = admin(&store);
    let admin_me = store.validate_session(&admin_session.token).unwrap();
    let token = store
        .create_enrolment(
            &admin_me,
            &lb_auth::NewEnrolment {
                purpose: lb_auth::EnrolmentPurpose::NewUser,
                role: Some(Role::Bookkeeper),
                user_id: None,
                company_id: None,
                display_name: None,
                ttl_minutes: None,
            },
        )
        .unwrap();
    let (bob, bob_key, bob_key_id, bob_first) = signed_in(&store, &token.token);
    let bob_second = sign_in(&store, &bob_key, bob_key_id).unwrap();
    let bob_me = store.validate_session(&bob_second.token).unwrap();

    // Bob can't touch the admin's session (and isn't told it exists).
    let err = store
        .revoke_session(&bob_me, admin_me.session_id)
        .unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }), "{err:?}");
    // Bob ends his first session from his second.
    store.revoke_session(&bob_me, bob_first.info.id).unwrap();
    assert!(store.validate_session(&bob_first.token).is_err());
    store.validate_session(&bob_second.token).unwrap();
    // The admin signs Bob out everywhere.
    assert_eq!(store.revoke_user_sessions(&admin_me, bob.id).unwrap(), 1);
    assert!(store.validate_session(&bob_second.token).is_err());
    // Bob can't sign the admin out everywhere.
    let err = store
        .revoke_user_sessions(&bob_me, admin_me.user_id)
        .unwrap_err();
    assert!(matches!(err, AuthError::Forbidden(_)), "{err:?}");
}

#[test]
fn disabling_a_user_ends_their_sessions_and_stops_sign_in() {
    let (store, _clock) = store();
    let (_, admin_session) = admin(&store);
    let admin_me = store.validate_session(&admin_session.token).unwrap();
    let token = store
        .create_enrolment(
            &admin_me,
            &lb_auth::NewEnrolment {
                purpose: lb_auth::EnrolmentPurpose::NewUser,
                role: None,
                user_id: None,
                company_id: None,
                display_name: None,
                ttl_minutes: None,
            },
        )
        .unwrap();
    let (bob, bob_key, bob_key_id, bob_session) = signed_in(&store, &token.token);

    let disabled = store
        .update_user(
            &admin_me,
            bob.id,
            &UserUpdate {
                disabled: Some(true),
                ..UserUpdate::default()
            },
        )
        .unwrap();
    assert!(disabled.disabled_at.is_some());
    let err = store.validate_session(&bob_session.token).unwrap_err();
    assert!(matches!(err, AuthError::Unauthenticated), "{err:?}");
    let err = sign_in(&store, &bob_key, bob_key_id).unwrap_err();
    assert!(matches!(err, AuthError::UserDisabled), "{err:?}");

    store
        .update_user(
            &admin_me,
            bob.id,
            &UserUpdate {
                disabled: Some(false),
                ..UserUpdate::default()
            },
        )
        .unwrap();
    let again = sign_in(&store, &bob_key, bob_key_id).unwrap();
    store.validate_session(&again.token).unwrap();
}

#[test]
fn local_sessions_need_no_sign_in_and_have_their_own_lifetimes() {
    let (store, clock) = store();
    let owner = store.create_user("Desk Owner", Role::Admin).unwrap();
    let session = store
        .create_local_session(owner.id, Some("LibreBooks desktop"))
        .unwrap();
    let me = store.validate_session(&session.token).unwrap();
    assert_eq!(me.kind, lb_auth::SessionKind::Local);
    assert_eq!(me.user_id, owner.id);
    clock.advance(days(1) + Duration::from_secs(1));
    assert!(store.validate_session(&session.token).is_err());
}

#[test]
fn sessions_survive_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let clock = lb_auth::Clock::manual(start_time());
    let token = {
        let store = AuthStore::open(&path, config(&clock)).unwrap();
        let (_, session) = admin(&store);
        session.token
    };
    let store = AuthStore::open(&path, config(&clock)).unwrap();
    let me = store.validate_session(&token).unwrap();
    assert_eq!(me.role, Role::Admin);
    assert!(store.bootstrap().unwrap().is_none());
}

#[test]
fn prune_forgets_long_dead_sessions_and_tokens() {
    let (store, clock) = store();
    let (user, session) = admin(&store);
    store.logout(&session.token).unwrap();
    store.prune().unwrap();
    // Still within 30 days: kept (though not listed, since it's ended).
    clock.advance(days(31));
    store.prune().unwrap();
    assert_eq!(store.list_sessions(user.id, None).unwrap(), []);
    assert!(store.validate_session(&session.token).is_err());
}
