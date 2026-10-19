//! The normaliser's [`Resolver`], backed by the company file: QBD ids are found
//! through the store's external references, inside the same SQLite transaction
//! the record is saved in. Lists the bridge doesn't expose (sales tax codes and
//! items, customer/vendor/job types) are created the first time a record names
//! them, and roll back with that record if it fails.

use lb_core::lists::{SalesTaxCode, TypeEntry, TypeListKind};
use lb_core::{
    AccountId, ClassId, EntityId, EntityKind, Item, ItemId, ItemType, PaymentMethodId, SalesRepId,
    SalesTaxCodeId, TermsId, TxnId, TypeId,
};
use lb_store::{StoreError, Writer, codec, entities, extref, items, lists};
use rust_decimal::Decimal;

use crate::entity::QbdEntity;
use crate::qbxml::{NormaliseError, QbRef, Resolver};

/// External-reference kinds for records the import creates itself.
pub const KIND_SALES_TAX_CODE: &str = "SalesTaxCode";
pub const KIND_SALES_TAX_ITEM: &str = "ItemSalesTax";
pub const KIND_TAX_AGENCY: &str = "PlaceholderTaxAgency";

/// The vendor sales tax items created from transactions are owed to. QBD knows
/// the real agency, but the bridge doesn't list sales tax items, so LibreBooks
/// can't see it.
pub const PLACEHOLDER_AGENCY_NAME: &str = "Sales tax agency (from QuickBooks)";

/// State that outlives one record: what the company's sales tax preferences
/// say, and what has been created so far (for the report).
#[derive(Debug, Default)]
pub struct ResolverState {
    pub taxable_code: Option<String>,
    pub non_taxable_code: Option<String>,
    pub created: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug)]
pub struct StoreResolver<'a, 'w> {
    pub w: &'a mut Writer<'w>,
    pub source: &'a str,
    pub state: &'a mut ResolverState,
    /// Names created inside this record's transaction; moved into
    /// `state.created` only if the record commits.
    pub created_here: Vec<String>,
}

fn store_err(e: &StoreError) -> NormaliseError {
    NormaliseError::Resolver(format!("the books couldn't be read or written: {e}"))
}

impl<'a, 'w> StoreResolver<'a, 'w> {
    pub fn new(w: &'a mut Writer<'w>, source: &'a str, state: &'a mut ResolverState) -> Self {
        StoreResolver {
            w,
            source,
            state,
            created_here: Vec::new(),
        }
    }

    /// The local id behind a reference, looked up under exactly one external
    /// kind. QBD `ListID`s are *not* unique across lists (the user's Terms and
    /// Payment Methods share five), so a lookup must always say which list.
    fn local<T: std::str::FromStr>(
        &self,
        kind: &str,
        r: &QbRef,
    ) -> Result<Option<T>, NormaliseError> {
        match &r.list_id {
            Some(id) => self.local_of_kind(kind, id),
            None => Ok(None),
        }
    }

    fn local_by_id<T: std::str::FromStr>(
        &self,
        external_id: &str,
        local_kind: &str,
    ) -> Result<Option<T>, NormaliseError> {
        let found = extref::get_any_kind(&self.w.tx, self.source, external_id)
            .map_err(|e| store_err(&e))?;
        Ok(found
            .filter(|x| x.local_kind == local_kind)
            .and_then(|x| codec::parse_id(&x.local_id).ok()))
    }

    fn local_of_kind<T: std::str::FromStr>(
        &self,
        kind: &str,
        external_id: &str,
    ) -> Result<Option<T>, NormaliseError> {
        let found =
            extref::get(&self.w.tx, self.source, kind, external_id).map_err(|e| store_err(&e))?;
        Ok(found.and_then(|x| codec::parse_id(&x.local_id).ok()))
    }

    fn put(
        &mut self,
        kind: &str,
        external_id: &str,
        local_kind: &str,
        local_id: &str,
    ) -> Result<(), NormaliseError> {
        extref::put(
            self.w,
            self.source,
            kind,
            external_id,
            local_kind,
            local_id,
            None,
            None,
        )
        .map_err(|e| store_err(&e))
    }

    fn is_taxable_code(&self, name: &str) -> bool {
        if self.state.taxable_code.as_deref() == Some(name) {
            return true;
        }
        if self.state.non_taxable_code.as_deref() == Some(name) {
            return false;
        }
        // QBD ships "Tax" (taxable) and "Non"; the codes companies add are
        // overwhelmingly exemptions (OOS, GOV, RES, WHL…). Sales tax computed with
        // a wrong guess shows up as a mismatch in the sync report.
        name.eq_ignore_ascii_case("Tax")
    }

    fn placeholder_agency(&mut self) -> Result<EntityId, NormaliseError> {
        if let Some(id) = self.local_of_kind(KIND_TAX_AGENCY, "agency")? {
            return Ok(id);
        }
        let existing = entities::by_full_name(&self.w.tx, PLACEHOLDER_AGENCY_NAME)
            .map_err(|e| store_err(&e))?;
        let id = match existing {
            Some(e) if e.kind == EntityKind::Vendor => e.id,
            _ => {
                let mut v = lb_core::Entity::blank(EntityKind::Vendor, PLACEHOLDER_AGENCY_NAME);
                v.notes = Some(
                    "Created by the QuickBooks import: the bridge doesn't list sales tax items, so the agency each one is owed to isn't known.".into(),
                );
                let saved = entities::save(self.w, v).map_err(|e| store_err(&e))?;
                self.created_here
                    .push(format!("vendor \"{PLACEHOLDER_AGENCY_NAME}\""));
                saved.id
            }
        };
        self.put(KIND_TAX_AGENCY, "agency", "entity", &id.to_string())?;
        Ok(id)
    }
}

impl Resolver for StoreResolver<'_, '_> {
    fn account(&mut self, r: &QbRef) -> Result<Option<AccountId>, NormaliseError> {
        self.local("Account", r)
    }

    /// A name reference may point into any of the four name lists
    /// (`PayeeEntityRef`, a journal line's `EntityRef`). If the id is imported in
    /// more than one, the full name decides.
    fn entity(&mut self, r: &QbRef) -> Result<Option<EntityId>, NormaliseError> {
        let mut found: Vec<EntityId> = Vec::new();
        for kind in ["Customer", "Vendor", "Employee", "OtherName"] {
            if let Some(id) = self.local(kind, r)? {
                found.push(id);
            }
        }
        if found.len() <= 1 {
            return Ok(found.pop());
        }
        for id in found {
            let e = entities::find(&self.w.tx, id).map_err(|e| store_err(&e))?;
            if e.is_some_and(|e| Some(&e.full_name) == r.full_name.as_ref()) {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn item(&mut self, r: &QbRef) -> Result<Option<ItemId>, NormaliseError> {
        match self.local("Item", r)? {
            Some(id) => Ok(Some(id)),
            None => self.local(KIND_SALES_TAX_ITEM, r),
        }
    }

    fn class(&mut self, r: &QbRef) -> Result<Option<ClassId>, NormaliseError> {
        self.local("Class", r)
    }

    fn terms(&mut self, r: &QbRef) -> Result<Option<TermsId>, NormaliseError> {
        self.local("Terms", r)
    }

    fn payment_method(&mut self, r: &QbRef) -> Result<Option<PaymentMethodId>, NormaliseError> {
        self.local("PaymentMethod", r)
    }

    fn sales_rep(&mut self, r: &QbRef) -> Result<Option<SalesRepId>, NormaliseError> {
        self.local("SalesRep", r)
    }

    fn sales_tax_code(&mut self, r: &QbRef) -> Result<Option<SalesTaxCodeId>, NormaliseError> {
        let Some(name) = r.full_name.clone() else {
            return Ok(None);
        };
        let key = r.list_id.clone().unwrap_or_else(|| format!("name:{name}"));
        if let Some(id) = self.local_of_kind(KIND_SALES_TAX_CODE, &key)? {
            return Ok(Some(id));
        }
        let existing: Option<SalesTaxCode> =
            lists::by_name(&self.w.tx, &name).map_err(|e| store_err(&e))?;
        let id = if let Some(c) = existing {
            c.id
        } else {
            let code = SalesTaxCode {
                id: SalesTaxCodeId::new(),
                is_taxable: self.is_taxable_code(&name),
                name: name.clone(),
                description: None,
                is_active: true,
                edit_seq: 0,
            };
            let saved = lists::save(self.w, code).map_err(|e| store_err(&e))?;
            self.created_here.push(format!(
                "sales tax code \"{name}\" ({})",
                if saved.is_taxable {
                    "taxable"
                } else {
                    "non-taxable"
                }
            ));
            saved.id
        };
        self.put(KIND_SALES_TAX_CODE, &key, "list", &id.to_string())?;
        Ok(Some(id))
    }

    fn sales_tax_item(
        &mut self,
        r: &QbRef,
        rate: Option<Decimal>,
    ) -> Result<Option<ItemId>, NormaliseError> {
        // A sales tax item imported through the item list (the Web Connector
        // transport lists them) wins.
        if let Some(id) = self.local("Item", r)? {
            return Ok(Some(id));
        }
        let Some(name) = r.full_name.clone() else {
            return Ok(None);
        };
        let key = r.list_id.clone().unwrap_or_else(|| format!("name:{name}"));
        if let Some(id) = self.local_of_kind(KIND_SALES_TAX_ITEM, &key)? {
            return Ok(Some(id));
        }
        let existing = items::by_full_name(&self.w.tx, &name).map_err(|e| store_err(&e))?;
        let id = match existing {
            Some(i) if matches!(i.item_type, ItemType::SalesTax | ItemType::SalesTaxGroup) => i.id,
            Some(_) => {
                return Err(NormaliseError::Unsupported(format!(
                    "the sales tax item \"{name}\" has the same name as another kind of item"
                )));
            }
            None => {
                let agency = self.placeholder_agency()?;
                let mut item = Item::blank(ItemType::SalesTax, &name);
                item.tax_rate = rate;
                item.tax_agency_id = Some(agency);
                item.sales_description = Some(
                    "Created by the QuickBooks import from a transaction that charged it".into(),
                );
                let saved = items::save(self.w, item).map_err(|e| store_err(&e))?;
                self.created_here.push(format!(
                    "sales tax item \"{name}\" at {}%",
                    rate.unwrap_or_default()
                ));
                saved.id
            }
        };
        self.put(KIND_SALES_TAX_ITEM, &key, "item", &id.to_string())?;
        Ok(Some(id))
    }

    fn type_entry(
        &mut self,
        list: TypeListKind,
        r: &QbRef,
    ) -> Result<Option<TypeId>, NormaliseError> {
        let Some(full) = r.full_name.clone() else {
            return Ok(None);
        };
        let kind = match list {
            TypeListKind::CustomerType => "CustomerType",
            TypeListKind::VendorType => "VendorType",
            TypeListKind::JobType => "JobType",
        };
        let key = r.list_id.clone().unwrap_or_else(|| format!("name:{full}"));
        if let Some(id) = self.local_of_kind(kind, &key)? {
            return Ok(Some(id));
        }
        // Types are hierarchical (`Retail:Online`): make sure each level exists.
        let mut parent: Option<TypeId> = None;
        let mut so_far = String::new();
        for part in full.split(':') {
            if !so_far.is_empty() {
                so_far.push(':');
            }
            so_far.push_str(part);
            let existing: Option<TypeEntry> =
                lists::by_name(&self.w.tx, &so_far).map_err(|e| store_err(&e))?;
            parent = Some(match existing {
                Some(t) if t.list == list => t.id,
                Some(_) => {
                    // LibreBooks keeps all three type lists in one namespace.
                    self.state.notes.push(format!(
                        "the {kind} \"{so_far}\" has the same name as another list's type, so it wasn't linked"
                    ));
                    return Ok(None);
                }
                None => {
                    let t = TypeEntry {
                        id: TypeId::new(),
                        list,
                        name: part.to_owned(),
                        full_name: so_far.clone(),
                        parent_id: parent,
                        is_active: true,
                        edit_seq: 0,
                    };
                    let saved = lists::save(self.w, t).map_err(|e| store_err(&e))?;
                    self.created_here.push(format!("{kind} \"{so_far}\""));
                    saved.id
                }
            });
        }
        let Some(id) = parent else {
            return Ok(None);
        };
        self.put(kind, &key, "list", &id.to_string())?;
        Ok(Some(id))
    }

    fn txn(
        &mut self,
        txn_type: Option<&str>,
        txn_id: &str,
    ) -> Result<Option<TxnId>, NormaliseError> {
        match txn_type.and_then(QbdEntity::for_qbxml_txn_type) {
            Some(entity) => self.local_of_kind(entity.name(), txn_id),
            // QBD's TxnIDs come from one company-wide counter, so without a type
            // any imported transaction with this id is the one.
            None => self.local_by_id(txn_id, "txn"),
        }
    }
}
