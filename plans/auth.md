# lb-auth: authentication for the server

## Goal

`crates/lb-auth` is a pure library (no HTTP) that `lb-server` maps onto the
`/auth/*`, `/session` and `/users` endpoints in `plans/api.md`. The user's
requirement: **all remote auth is passkeys (or something equally secure that
isn't a password), plus a simple enrolment token that expires after a short
window or on first use.** Desktop thin clients whose webview lacks WebAuthn
(Linux WebKitGTK) use **device keys** (Ed25519) enrolled with the same tokens.

## Environment / context

- Built on an agent worktree branch for lb-auth (merged by the parent session).
- Its own SQLite file (`server.db`), separate from company files.
  `PRAGMA application_id` = `0x4C426175` ("LBau").
- Build/test through the CPU broker:
  `node ~/.claude/bin/cpu-slots.mjs run --slots 4 --label "librebooks lb-auth" -- cargo test -p lb-auth`.
- TypeScript types export to `web/src/api/generated/` when the tests run
  (ts-rs writes LF; convert to CRLF before committing to match the checkout).

## Public API (for lb-server)

`AuthStore::open(path, AuthConfig)` / `AuthStore::in_memory(AuthConfig)`;
`Clone + Send + Sync`. Every network-facing method takes the request's
`ClientInfo { ip, user_agent }` (rate limit key, session label) and, where
WebAuthn binds to it, the request's `Origin`. Errors are `AuthError`, whose
`code()` is the API error code (`invalid`, `unauthenticated`, `forbidden`,
`not_found`, `conflict`, `rate_limited`, `internal`).

| Endpoint | Method |
| --- | --- |
| startup | `bootstrap() -> Option<MintedEnrolment>` (print `token` as a link) |
| `POST /auth/enroll/start` | `start_enrolment(&EnrolStartRequest, origin, client) -> EnrolStart` |
| `POST /auth/enroll/finish` | `finish_enrolment(&EnrolFinish, origin, client) -> (User, NewSession)` |
| `POST /auth/login/start` | `start_login(origin, client) -> LoginStart` |
| `POST /auth/login/finish` | `finish_login(&LoginFinish, origin, client) -> (User, NewSession)` |
| `POST /auth/device/challenge` | `device_challenge(client) -> DeviceChallenge` |
| `POST /auth/device/login` | `device_login(&DeviceLogin, origin, client) -> (User, NewSession)` |
| `POST /auth/device/enroll` | `enrol_device(&DeviceEnrol, client) -> (User, DeviceKeyInfo)` |
| every request | `validate_session(token) -> Principal` |
| `POST /auth/logout` | `logout(token)` |
| `GET/DELETE /auth/passkeys` | `list_passkeys(user)`, `delete_passkey(by, id)`, `rename_passkey(by, id, name)` |
| `GET/DELETE /auth/sessions` | `list_sessions(user, current)`, `revoke_session(by, id)`, `revoke_user_sessions(by, user)` |
| `POST /auth/enrollments` | `create_enrolment(by, &NewEnrolment) -> MintedEnrolment` (server adds `url`) |
| admin lists | `list_enrolments()`, `revoke_enrolment(by, id)`, `inspect_enrolment(token, client)` |
| `GET/PATCH/DELETE /users` | `list_users()`, `user(id)`, `update_user(by, id, &UserUpdate)`, `delete_user(by, id)` |
| companies | `grant_company_access`, `revoke_company_access`, `company_access(user)`, `company_role(p, company)`, `authorize_company(p, company, Permission)` |
| CLI / desktop | `operator_enrolment(&NewEnrolment)`, `create_user(name, role)`, `create_local_session(user, label)` |
| housekeeping | `prune()` (call now and then) |

Also `Principal::can/require(Permission)`, `Role::allows(Permission)`,
`device_login_message(challenge, origin)` (the desktop app signs this),
`RelyingParty::for_origin(url, name)`, `Clock::manual(t)` for tests.

**Contract additions to fold into `plans/api.md`** (not edited from this
branch, which only owns lb-auth):

1. `POST /auth/enroll/start` body is `EnrolStartRequest { token, display_name? }`.
   The name labels the passkey in the person's password manager (WebAuthn
   `user.name`/`displayName` can't be changed later), so the page should ask
   for it *before* the passkey prompt. Finish falls back to it.
2. `POST /auth/device/enroll` body also takes `display_name?` (new users) and
   returns the user as well as the key.
3. A `finish_enrolment` that fails with `invalid` "please enter your name"
   keeps the ceremony: the page asks and resubmits the same credential.
4. Device login needs the origin the client connected to: from `Origin`, or the
   server's public URL when a non-browser client sends none.

## Decisions (don't re-ask)

1. **WebAuthn library: `webauthn_rp` 0.3** (pure Rust, RustCrypto; MIT/Apache),
   feature `serde_relaxed`. Evaluated by reading its source, then proven by
   the round-trip tests:
   - Strict L3 implementation (CTAP2 canonical CBOR, every registration and
     authentication step, interrelated-data checks), `none` attestation only,
     which is what we want.
   - Its client-state types serialise to `PublicKeyCredentialCreationOptionsJSON`
     / `PublicKeyCredentialRequestOptionsJSON`, which `@simplewebauthn/browser`
     v14 takes as `optionsJSON` unchanged. We only patch `rp.name` (it uses the
     RP ID as the name).
   - `Registration::from_json_relaxed` / `DiscoverableAuthentication16::from_json_relaxed`
     read the `RegistrationResponseJSON` / `AuthenticationResponseJSON` that
     SimpleWebAuthn returns (L3 extras `publicKey`, `publicKeyAlgorithm`,
     `authenticatorData`, `transports` included).
   - Signature counters: `prev == 0 || new > prev` passes, so 0-counter passkeys
     (Apple, Google, Windows Hello) work; a regression on a counting
     authenticator fails (likely clone). We use `SignatureCounterEnforcement::Fail`.
   - Credentials persist through its `bin` encoding (`StaticState` = public key,
     `DynamicState` = UV/backup/counter/attachment, 7 bytes), beside readable
     columns.
   - Options chosen: `PublicKeyCredentialCreationOptions::passkey` (resident key
     required, UV required, credProtect UV-required, ES256/EdDSA/ES384/RS256)
     and `DiscoverableCredentialRequestOptions::passkey` (UV required, empty
     allowCredentials, so usernameless and conditional UI). Verification:
     the RP's exact origins, no cross-origin iframes, unsolicited extensions
     tolerated, authenticator attachment allowed to change (`Update(false)`),
     backup state follows the stored one.
   - Costs: it pulls older RustCrypto majors (rand 0.9, ed25519-dalek 2,
     p256/p384 0.13, rsa 0.9) beside the workspace's newer ones. Accepted. The
     `rsa` crate's Marvin timing advisory (RUSTSEC-2023-0071) concerns private-key
     operations; we only verify, so it does not apply.
   - Hand-rolling the RP verification on RustCrypto was the fallback; not needed.
2. **No OpenSSL.** `cargo tree -i openssl-sys --workspace` →
   "did not match any packages" (2026-10-19).
3. **Timestamps in `server.db` are INTEGER Unix milliseconds**, not RFC 3339 text
   as in company files: auth compares times in SQL (`expires_at > now`), and
   RFC 3339 strings with varying fractional digits do not sort correctly.
4. **Pending ceremonies (WebAuthn and device challenges) live in memory** with a
   TTL and a size cap (`max_pending_ceremonies`, default 10 000; over it →
   `Busy`), not in the database. They are minutes-lived, single use by
   construction (removed on finish), and losing them on restart only means
   "try again". `webauthn_rp`'s server state is deliberately not serialisable
   by default for the same reasons. A multi-process deployment would need
   sticky sessions; not a goal.
5. **Secrets stored only as SHA-256 hashes** (enrolment and session tokens).
   Tokens are high-entropy (160-bit enrolment, 256-bit session), so a plain hash
   is sufficient (no salt/KDF) and lookup by hash leaks nothing useful through
   timing. The token presented at enrolment finish is compared with the one the
   ceremony started with in constant time (`subtle`). A test greps the database
   file for the token bodies.
6. **Token formats**: enrolment `lbe_` + base64url(20 random bytes); session
   `lbs_` + base64url(32 random bytes). Prefixes make them recognisable in logs
   and to secret scanners. Randomness straight from the OS (`rand::rngs::SysRng`).
7. **Single use is atomic in SQLite**: redeeming is
   `UPDATE … SET used_at WHERE hash = ? AND used_at IS NULL AND revoked_at IS NULL AND expires_at > now`
   inside the same IMMEDIATE transaction that creates the user, credential and
   session; only `changes() == 1` succeeds. Tested with 4 threads racing, over
   one connection and over two connections to the same file.
8. **Relying parties**: a passkey is bound to an RP ID (a domain). Config holds
   several `RelyingParty { id, name, origins }`; the request's `Origin` selects
   the RP; a ceremony must finish at an origin of the RP it started on. A
   passkey registered at `books.example.com` cannot sign in at `localhost`, and
   vice versa: users enrol once per hostname they use. Browsers refuse
   IP-address RP IDs and require a secure context (HTTPS or `http://localhost`),
   so a bare LAN IP cannot use passkeys; such clients use a hostname, or device
   keys (`AuthConfig::extra_device_origins`). Config validation rejects IP RP
   IDs, non-HTTPS non-localhost origins, origins with paths, and origins whose
   host isn't the RP ID or a subdomain of it.
9. **Device-key login message**: `b"librebooks device login v1\0" || challenge(32) || origin`.
   Binding the origin the client connected to stops a malicious server from
   relaying our challenge and replaying the signature. Verified with
   `verify_strict`; weak (small-order) public keys refused at enrolment.
10. **Session defaults**: idle 14 days, absolute 90 days for passkey and device
    sessions; `last_seen_at` written at most once a minute. Local (desktop
    in-process) sessions: idle 1 day, absolute 30 days. Removing a passkey or
    device key ends the sessions it signed in; disabling a user ends all theirs
    and cancels their outstanding enrolment links.
11. **Roles**: `admin` (everything), `bookkeeper` (read/write books; no user or
    server admin), `viewer` (read only). Permissions: `ReadBooks`, `WriteBooks`,
    `AdminBooks`, `ManageUsers`, `ManageServer`. Admins see every company;
    others need a `company_access` row and get the lower of that grant and
    their server role. Anyone may mint an `add_device` token for themselves.
12. **Guards**: the last enabled admin cannot be disabled, demoted or deleted;
    nobody can delete themselves; a user cannot remove their own last sign-in
    method (admins removing someone else's are not stopped: lost devices).
13. **Bootstrap**: mints a `first_admin` token (TTL 1 h) only when no enabled
    admin exists, revoking older unused `first_admin` tokens so only the one
    just printed is live. Redeeming one fails (`AdminExists`) once an admin
    exists. Starting a passkey enrolment only checks a token; finishing redeems.
14. **Rate limiting**: in memory, per IP, failures only (bad/expired/used
    tokens, failed proofs, unknown credentials/challenges): 20 per 10 minutes,
    then `RateLimited { retry_after }` until the oldest ages out. A blocked IP
    can't start new ceremonies either.
15. **Time** comes from `AuthConfig::clock` (`Clock::system()` or
    `Clock::manual(t)` + `advance`) so expiry is tested without sleeping.
    webauthn_rp's own ceremony timeout uses the real `Instant` clock (same 5
    minutes); ours is checked first.

## Findings / gotchas

- `webauthn_rp`'s `PublicKeyCredentialUserEntity.name` must satisfy PRECIS
  UsernameCasePreserved (no spaces): "Ada Lovelace" fails, so we fall back to
  "Ada.Lovelace", then "librebooks". `displayName` (Nickname profile) keeps
  spaces.
- Its option builders borrow the RP ID, user handle and names, so serialise the
  client state to JSON before those go out of scope.
- `UserHandle::<16>::decode([u8; 16])` (bin feature) is the way to build a user
  handle from a UUID; there's no `From` without the `custom` feature.
- All the infallible `encode()`s return `Result<_, Infallible>`; `let Ok(x) = …`
  destructures them without an unwrap.
- Error messages from webauthn_rp are specific and safe to show
  (e.g. "the origin sent from the client is not an allowed origin", "user was
  not verified despite being required to"); they go out as `invalid` with the
  message prefixed "the passkey could not be verified".
- The test authenticator writes CBOR by hand in CTAP2 canonical order (COSE key
  1, 3, -1, -2, -3; attestation object `fmt`, `attStmt`, `authData`) because
  webauthn_rp rejects non-canonical CBOR. No `credProtect` output was needed
  in authData for registration to pass.
- The cpu-slots broker occasionally fails with `EPERM` opening its mutex file
  under contention; rerunning works.

## Progress log

- [x] Store, schema, config, errors
- [x] Enrolment tokens + bootstrap
- [x] Sessions
- [x] Passkeys (webauthn_rp)
- [x] Device keys
- [x] Roles / company access
- [x] Rate limiting
- [x] Tests: 71 (unit + integration: passkeys, devices, sessions, enrolment)
- [x] TS exports, clippy (pedantic) clean, fmt, OpenSSL check

## What's left / known gaps

- lb-server wiring (cookie, CSRF header, origin extraction, `url` on minted
  tokens) and the api.md contract additions above.
- Not tested against a real browser yet; the hand-built authenticator mirrors
  SimpleWebAuthn v14's JSON. First real-browser run should be an e2e test in
  lb-server/web (Playwright has a virtual authenticator via CDP).
- No WebAuthn Signal API use (`signalUnknownCredential` after a deleted
  passkey, `signalCurrentUserDetails` after a rename); the UI can add it.
- Optional Cloudflare Access JWT check (master plan, decision 8) belongs in
  lb-server/lb-tunnel, not here.
- Rate limiting is per process and per IP; behind a proxy the server must pass
  the real client IP in `ClientInfo`.

## Open questions for the user

None.

## Things not to do

- Don't add `webauthn-rs` (OpenSSL).
- Don't store a token or session secret in clear, or log it (`NewSession` and
  `MintedEnrolment` hide the token in `Debug`).
- Don't wire `operator_enrolment` or `create_local_session` to the network.
