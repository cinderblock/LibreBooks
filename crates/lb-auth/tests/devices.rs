//! Device keys: enrol with a token, sign in by signing a challenge.

mod common;

use common::*;
use ed25519_dalek::{Signer as _, SigningKey};
use lb_auth::{
    AuthError, AuthStore, DeviceEnrol, DeviceKeyInfo, DeviceLogin, EnrolmentPurpose, NewEnrolment,
    Principal, Role, SessionKind, User, device_login_message,
};

struct Device {
    key: SigningKey,
}

impl Device {
    fn new() -> Device {
        Device {
            key: SigningKey::from_bytes(&rand::random::<[u8; 32]>()),
        }
    }

    fn public_key(&self) -> String {
        b64(self.key.verifying_key().as_bytes())
    }

    fn enrol(
        &self,
        store: &AuthStore,
        token: &str,
        display_name: Option<&str>,
    ) -> Result<(User, DeviceKeyInfo), AuthError> {
        store.enrol_device(
            &DeviceEnrol {
                token: token.to_owned(),
                public_key: self.public_key(),
                name: "Test laptop".to_owned(),
                display_name: display_name.map(str::to_owned),
            },
            &client(),
        )
    }

    /// Sign in, signing for `signed_origin` and presenting at `origin`.
    fn login_as(
        &self,
        store: &AuthStore,
        key_id: lb_auth::DeviceKeyId,
        signed_origin: &str,
        origin: &str,
    ) -> Result<Principal, AuthError> {
        let challenge = store.device_challenge(&client())?;
        let bytes: [u8; 32] = unb64(&challenge.challenge).try_into().unwrap();
        let signature = self.key.sign(&device_login_message(&bytes, signed_origin));
        let (_, session) = store.device_login(
            &DeviceLogin {
                challenge_id: challenge.challenge_id,
                key_id,
                signature: b64(&signature.to_bytes()),
            },
            origin,
            &client(),
        )?;
        store.validate_session(&session.token)
    }

    fn login(
        &self,
        store: &AuthStore,
        key_id: lb_auth::DeviceKeyId,
    ) -> Result<Principal, AuthError> {
        self.login_as(store, key_id, ORIGIN, ORIGIN)
    }
}

#[test]
fn first_admin_enrols_a_device_and_signs_in() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    let device = Device::new();
    let (user, key) = device.enrol(&store, &token.token, Some("Linus")).unwrap();
    assert_eq!(user.role, Role::Admin);
    assert_eq!(user.display_name, "Linus");
    assert_eq!(key.user_id, user.id);

    let me = device.login(&store, key.id).unwrap();
    assert_eq!(me.user_id, user.id);
    assert_eq!(me.kind, SessionKind::Device);
    let sessions = store.list_sessions(user.id, Some(me.session_id)).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].current);
    assert_eq!(sessions[0].label.as_deref(), Some("Test laptop"));
    assert!(
        store.list_device_keys(user.id).unwrap()[0]
            .last_used_at
            .is_some()
    );

    // The token is spent.
    let err = Device::new().enrol(&store, &token.token, None).unwrap_err();
    assert!(matches!(err, AuthError::TokenUsed), "{err:?}");
}

#[test]
fn device_challenges_are_single_use_bound_to_origin_and_expire() {
    let (store, clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    let device = Device::new();
    let (_, key) = device.enrol(&store, &token.token, None).unwrap();

    // Replay: the same challenge and signature twice.
    let challenge = store.device_challenge(&client()).unwrap();
    let bytes: [u8; 32] = unb64(&challenge.challenge).try_into().unwrap();
    let login = DeviceLogin {
        challenge_id: challenge.challenge_id,
        key_id: key.id,
        signature: b64(&device
            .key
            .sign(&device_login_message(&bytes, ORIGIN))
            .to_bytes()),
    };
    store.device_login(&login, ORIGIN, &client()).unwrap();
    let err = store.device_login(&login, ORIGIN, &client()).unwrap_err();
    assert!(matches!(err, AuthError::ChallengeNotFound), "{err:?}");

    // A signature for another origin doesn't verify here.
    let err = device
        .login_as(&store, key.id, "https://evil.example.net", ORIGIN)
        .unwrap_err();
    assert!(matches!(err, AuthError::BadSignature), "{err:?}");
    // An origin the server doesn't serve is refused outright.
    let err = device
        .login_as(
            &store,
            key.id,
            "https://evil.example.net",
            "https://evil.example.net",
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::UnknownOrigin { .. }), "{err:?}");
    // Device-only origins (a LAN IP) work for device keys.
    device
        .login_as(&store, key.id, LAN_ORIGIN, LAN_ORIGIN)
        .unwrap();

    // Someone else's key can't sign for this device.
    let err = Device {
        key: Device::new().key,
    }
    .login(&store, key.id)
    .unwrap_err();
    assert!(matches!(err, AuthError::BadSignature), "{err:?}");

    // A challenge answered too late.
    let challenge = store.device_challenge(&client()).unwrap();
    let bytes: [u8; 32] = unb64(&challenge.challenge).try_into().unwrap();
    clock.advance(minutes(3));
    let err = store
        .device_login(
            &DeviceLogin {
                challenge_id: challenge.challenge_id,
                key_id: key.id,
                signature: b64(&device
                    .key
                    .sign(&device_login_message(&bytes, ORIGIN))
                    .to_bytes()),
            },
            ORIGIN,
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::ChallengeNotFound), "{err:?}");

    // Garbage signatures are a failed signature, not a crash.
    let challenge = store.device_challenge(&client()).unwrap();
    let err = store
        .device_login(
            &DeviceLogin {
                challenge_id: challenge.challenge_id,
                key_id: key.id,
                signature: "AAAA".to_owned(),
            },
            ORIGIN,
            &client(),
        )
        .unwrap_err();
    assert!(matches!(err, AuthError::BadSignature), "{err:?}");
}

#[test]
fn revoking_a_device_key_ends_its_sessions_and_sign_ins() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    let laptop = Device::new();
    let (user, laptop_key) = laptop.enrol(&store, &token.token, None).unwrap();
    let me = laptop.login(&store, laptop_key.id).unwrap();

    // My only device: I can't revoke it myself.
    let err = store.revoke_device_key(&me, laptop_key.id).unwrap_err();
    assert!(matches!(err, AuthError::Conflict(_)), "{err:?}");

    // Add a second device, then revoke the first.
    let add = store
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
    let desktop = Device::new();
    let (same_user, desktop_key) = desktop.enrol(&store, &add.token, None).unwrap();
    assert_eq!(same_user.id, user.id);
    let on_desktop = desktop.login(&store, desktop_key.id).unwrap();
    let laptop_session = laptop.login(&store, laptop_key.id).unwrap();

    store.revoke_device_key(&on_desktop, laptop_key.id).unwrap();
    let ids: Vec<_> = store
        .list_sessions(user.id, None)
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert!(!ids.contains(&laptop_session.session_id));
    assert!(!ids.contains(&me.session_id));
    assert!(ids.contains(&on_desktop.session_id));
    let err = laptop.login(&store, laptop_key.id).unwrap_err();
    assert!(matches!(err, AuthError::UnknownDeviceKey), "{err:?}");
    assert_eq!(store.list_device_keys(user.id).unwrap().len(), 1);
}

#[test]
fn bad_public_keys_and_duplicate_devices_are_refused_without_spending_the_token() {
    let (store, _clock) = store();
    let token = store.bootstrap().unwrap().unwrap();
    for bad in ["", "not base64!", &b64(&[1u8; 31]), &b64(&[0u8; 32])] {
        let err = store
            .enrol_device(
                &DeviceEnrol {
                    token: token.token.clone(),
                    public_key: bad.to_owned(),
                    name: "x".to_owned(),
                    display_name: None,
                },
                &client(),
            )
            .unwrap_err();
        assert!(matches!(err, AuthError::Invalid(_)), "{bad:?}: {err:?}");
    }
    let device = Device::new();
    let (_, _) = device.enrol(&store, &token.token, None).unwrap();

    let me_token = store
        .operator_enrolment(&NewEnrolment {
            purpose: EnrolmentPurpose::NewUser,
            role: Some(Role::Bookkeeper),
            user_id: None,
            company_id: None,
            display_name: Some("Bob".to_owned()),
            ttl_minutes: None,
        })
        .unwrap();
    let err = device.enrol(&store, &me_token.token, None).unwrap_err();
    assert!(matches!(err, AuthError::AlreadyRegistered), "{err:?}");
    // The failed attempt rolled back: no Bob, and the token still works.
    assert_eq!(store.list_users().unwrap().len(), 1);
    let (bob, _) = Device::new().enrol(&store, &me_token.token, None).unwrap();
    assert_eq!(bob.display_name, "Bob");
    assert_eq!(bob.role, Role::Bookkeeper);
}

/// Exactly one of many threads racing to redeem one token wins, through one
/// store (one connection) or through two stores (two connections, as two
/// processes would have).
#[test]
fn a_token_is_redeemed_once_under_concurrency() {
    for two_connections in [false, true] {
        for _ in 0..10 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("server.db");
            let clock = lb_auth::Clock::manual(start_time());
            let a = AuthStore::open(&path, config(&clock)).unwrap();
            let b = if two_connections {
                AuthStore::open(&path, config(&clock)).unwrap()
            } else {
                a.clone()
            };
            let token = a.bootstrap().unwrap().unwrap();
            let barrier = std::sync::Barrier::new(4);
            let results: Vec<_> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..4)
                    .map(|i| {
                        let store = if i % 2 == 0 { a.clone() } else { b.clone() };
                        let token = token.token.clone();
                        let barrier = &barrier;
                        s.spawn(move || {
                            let device = Device::new();
                            barrier.wait();
                            device.enrol(&store, &token, Some("Racer")).map(|_| ())
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let won = results.iter().filter(|r| r.is_ok()).count();
            assert_eq!(won, 1, "two_connections={two_connections}: {results:?}");
            for r in &results {
                if let Err(e) = r {
                    assert!(matches!(e, AuthError::TokenUsed), "{e:?}");
                }
            }
            assert_eq!(a.list_users().unwrap().len(), 1);
        }
    }
}
