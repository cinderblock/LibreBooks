//! Who may do what.
//!
//! Three roles, deliberately coarse (QBD's per-area permissions can come
//! later if anyone needs them):
//!
//! | Permission | viewer | bookkeeper | admin |
//! | --- | :-: | :-: | :-: |
//! | `ReadBooks`: lists, registers, reports | ✓ | ✓ | ✓ |
//! | `WriteBooks`: enter and change transactions and lists | | ✓ | ✓ |
//! | `AdminBooks`: closing date, imports, restore, company settings | | | ✓ |
//! | `ManageUsers`: users, roles, enrolment links for others | | | ✓ |
//! | `ManageServer`: server settings, updates, tunnel, backups config | | | ✓ |
//!
//! Companies: an admin can open every company. Anyone else needs a
//! `company_access` grant, and their role in that company is the *lower* of
//! the grant and their server role, so demoting someone to viewer takes effect
//! everywhere at once.

use lb_core::{CompanyId, UserId};
use rusqlite::{OptionalExtension as _, params};

use crate::error::{AuthError, Result};
use crate::model::{CompanyAccess, Principal, Role};
use crate::{AuthStore, users};

/// Something a request wants to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    ReadBooks,
    WriteBooks,
    AdminBooks,
    ManageUsers,
    ManageServer,
}

impl Permission {
    /// For "you don't have permission to …".
    fn describe(self) -> &'static str {
        match self {
            Permission::ReadBooks => "see these books",
            Permission::WriteBooks => "change these books",
            Permission::AdminBooks => "administer this company",
            Permission::ManageUsers => "manage users",
            Permission::ManageServer => "change server settings",
        }
    }
}

impl Role {
    /// Whether this role may do `p`.
    pub fn allows(self, p: Permission) -> bool {
        match self {
            Role::Admin => true,
            Role::Bookkeeper => matches!(p, Permission::ReadBooks | Permission::WriteBooks),
            Role::Viewer => matches!(p, Permission::ReadBooks),
        }
    }
}

impl Principal {
    /// Whether this person may do `p` server-wide (not company-specific).
    pub fn can(&self, p: Permission) -> bool {
        self.role.allows(p)
    }

    /// `Ok` if this person may do `p` server-wide, `Forbidden` otherwise.
    pub fn require(&self, p: Permission) -> Result<()> {
        if self.can(p) {
            Ok(())
        } else {
            Err(AuthError::Forbidden(p.describe().to_owned()))
        }
    }
}

impl AuthStore {
    /// Give `user` access to `company` with `role` (replacing any earlier
    /// grant).
    pub fn grant_company_access(&self, user: UserId, company: CompanyId, role: Role) -> Result<()> {
        self.write(|tx| {
            users::get(tx, user)?;
            tx.execute(
                "INSERT INTO company_access (user_id, company_id, role) VALUES (?1, ?2, ?3)
                 ON CONFLICT (user_id, company_id) DO UPDATE SET role = excluded.role",
                params![user.to_string(), company.to_string(), role.as_str()],
            )?;
            Ok(())
        })
    }

    /// Take away `user`'s access to `company`.
    pub fn revoke_company_access(&self, user: UserId, company: CompanyId) -> Result<()> {
        self.write(|tx| {
            tx.execute(
                "DELETE FROM company_access WHERE user_id = ?1 AND company_id = ?2",
                params![user.to_string(), company.to_string()],
            )?;
            Ok(())
        })
    }

    /// The companies `user` has been granted, with the granted role (before
    /// capping by their server role; admins see every company regardless).
    pub fn company_access(&self, user: UserId) -> Result<Vec<CompanyAccess>> {
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT company_id, role FROM company_access WHERE user_id = ?1 ORDER BY company_id",
            )?;
            let rows = stmt.query_map([user.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (company, role) = row?;
                out.push(CompanyAccess {
                    company_id: company
                        .parse()
                        .map_err(|_| AuthError::Corrupt(format!("bad company id {company:?}")))?,
                    role: role.parse()?,
                });
            }
            Ok(out)
        })
    }

    /// `principal`'s effective role in `company`, or `None` if they can't open
    /// it at all.
    pub fn company_role(&self, principal: &Principal, company: CompanyId) -> Result<Option<Role>> {
        if principal.role == Role::Admin {
            return Ok(Some(Role::Admin));
        }
        let granted: Option<String> = self.read(|c| {
            Ok(c.query_row(
                "SELECT role FROM company_access WHERE user_id = ?1 AND company_id = ?2",
                params![principal.user_id.to_string(), company.to_string()],
                |r| r.get(0),
            )
            .optional()?)
        })?;
        granted
            .map(|g| g.parse::<Role>().map(|g| g.min(principal.role)))
            .transpose()
    }

    /// `Ok(role)` if `principal` may do `p` in `company`; `Forbidden`
    /// otherwise (including when they have no access to it at all, so a
    /// company's existence isn't confirmed to outsiders by a different error).
    pub fn authorize_company(
        &self,
        principal: &Principal,
        company: CompanyId,
        p: Permission,
    ) -> Result<Role> {
        match self.company_role(principal, company)? {
            Some(role) if role.allows(p) => Ok(role),
            _ => Err(AuthError::Forbidden(p.describe().to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_table() {
        use Permission::*;
        for p in [ReadBooks, WriteBooks, AdminBooks, ManageUsers, ManageServer] {
            assert!(Role::Admin.allows(p));
        }
        assert!(Role::Bookkeeper.allows(ReadBooks));
        assert!(Role::Bookkeeper.allows(WriteBooks));
        assert!(!Role::Bookkeeper.allows(AdminBooks));
        assert!(!Role::Bookkeeper.allows(ManageUsers));
        assert!(!Role::Bookkeeper.allows(ManageServer));
        assert!(Role::Viewer.allows(ReadBooks));
        assert!(!Role::Viewer.allows(WriteBooks));
        assert!(Role::Viewer < Role::Bookkeeper && Role::Bookkeeper < Role::Admin);
    }
}
