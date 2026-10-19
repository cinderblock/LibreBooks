//! The auth database's schema, as an ordered list of migrations.
//!
//! Times are INTEGER Unix milliseconds (UTC), not RFC 3339 text as in company
//! files: auth compares times in SQL (`expires_at > now`), and RFC 3339 strings
//! with different numbers of fractional digits do not sort as times do.
//!
//! Secrets (enrolment and session tokens) are stored only as SHA-256 hashes.
//!
//! Migrations only ever append. Never edit one that has shipped.

pub const MIGRATIONS: &[&str] = &[
    // 1: users, credentials, sessions, enrolment tokens, company access.
    r"
CREATE TABLE user (
    id            TEXT PRIMARY KEY,
    display_name  TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'bookkeeper', 'viewer')),
    created_at    INTEGER NOT NULL,
    disabled_at   INTEGER
) STRICT;

-- A WebAuthn credential. The opaque blobs are webauthn_rp's own encodings:
-- static_state is the public key (and registration-time extensions),
-- dynamic_state the UV/backup flags, sign counter and attachment it updates on
-- every sign-in. The readable columns beside them are for people and queries.
CREATE TABLE passkey (
    id               TEXT PRIMARY KEY,
    credential_id    BLOB NOT NULL UNIQUE,
    user_id          TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    rp_id            TEXT NOT NULL,
    user_handle      BLOB NOT NULL,
    static_state     BLOB NOT NULL,
    dynamic_state    BLOB NOT NULL,
    transports       INTEGER NOT NULL,
    sign_count       INTEGER NOT NULL,
    backup_eligible  INTEGER NOT NULL,
    backed_up        INTEGER NOT NULL,
    name             TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    last_used_at     INTEGER
) STRICT;
CREATE INDEX passkey_user ON passkey(user_id);

CREATE TABLE device_key (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    public_key    BLOB NOT NULL UNIQUE CHECK (length(public_key) = 32),
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_used_at  INTEGER,
    revoked_at    INTEGER
) STRICT;
CREATE INDEX device_key_user ON device_key(user_id);

-- credential_id is the passkey or device key that signed the session in, so
-- removing that credential ends its sessions too. NULL for local sessions.
CREATE TABLE session (
    id               TEXT PRIMARY KEY,
    token_hash       BLOB NOT NULL UNIQUE,
    user_id          TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL CHECK (kind IN ('passkey', 'device', 'local')),
    credential_id    TEXT,
    label            TEXT,
    ip               TEXT,
    created_at       INTEGER NOT NULL,
    last_seen_at     INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    idle_timeout_ms  INTEGER NOT NULL,
    revoked_at       INTEGER
) STRICT;
CREATE INDEX session_user ON session(user_id);
CREATE INDEX session_credential ON session(credential_id);

CREATE TABLE enrolment (
    id            TEXT PRIMARY KEY,
    token_hash    BLOB NOT NULL UNIQUE,
    purpose       TEXT NOT NULL CHECK (purpose IN ('first_admin', 'new_user', 'add_device')),
    role          TEXT CHECK (role IN ('admin', 'bookkeeper', 'viewer')),
    user_id       TEXT REFERENCES user(id) ON DELETE CASCADE,
    company_id    TEXT,
    display_name  TEXT,
    created_by    TEXT REFERENCES user(id) ON DELETE SET NULL,
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    used_at       INTEGER,
    used_by       TEXT REFERENCES user(id) ON DELETE SET NULL,
    revoked_at    INTEGER,
    CHECK (purpose <> 'add_device' OR user_id IS NOT NULL),
    CHECK (purpose = 'add_device' OR role IS NOT NULL)
) STRICT;

CREATE TABLE company_access (
    user_id     TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    company_id  TEXT NOT NULL,
    role        TEXT NOT NULL CHECK (role IN ('admin', 'bookkeeper', 'viewer')),
    PRIMARY KEY (user_id, company_id)
) STRICT;
",
];
