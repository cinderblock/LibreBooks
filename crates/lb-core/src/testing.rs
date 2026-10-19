//! An in-memory set of books implementing [`PostingContext`], for tests here and
//! in the crates above (reports, import), so they can exercise posting rules
//! without a database.

use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::account::{AccountRole, AccountType};
use crate::entity::EntityKind;
use crate::ids::{AccountId, EntityId, ItemId, SalesTaxCodeId, TxnId};
use crate::item::{Item, ItemType};
use crate::posting::{AccountInfo, OpenTxnInfo, PostingContext};

/// Accounts, names, items and open transactions, kept in maps.
#[derive(Debug, Default, Clone)]
pub struct MemoryBooks {
    pub accounts: HashMap<AccountId, (String, AccountInfo)>,
    pub roles: HashMap<AccountRole, AccountId>,
    pub entities: HashMap<EntityId, (String, EntityKind)>,
    pub items: HashMap<ItemId, Item>,
    pub tax_codes: HashMap<SalesTaxCodeId, bool>,
    pub open: HashMap<TxnId, OpenTxnInfo>,
    pub costs: HashMap<ItemId, Decimal>,
}

impl MemoryBooks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_account(&mut self, name: &str, account_type: AccountType) -> AccountId {
        let id = AccountId::new();
        self.accounts.insert(
            id,
            (
                name.to_owned(),
                AccountInfo {
                    account_type,
                    is_active: true,
                },
            ),
        );
        id
    }

    /// An account that also plays a role.
    pub fn add_role(&mut self, role: AccountRole) -> AccountId {
        let (t, name) = role.default_shape();
        let id = self.add_account(name, t);
        self.roles.insert(role, id);
        id
    }

    pub fn add_entity(&mut self, name: &str, kind: EntityKind) -> EntityId {
        let id = EntityId::new();
        self.entities.insert(id, (name.to_owned(), kind));
        id
    }

    pub fn add_item(&mut self, item: Item) -> ItemId {
        let id = item.id;
        self.items.insert(id, item);
        id
    }

    /// A service item selling into `income`.
    pub fn add_service(&mut self, name: &str, income: AccountId) -> ItemId {
        let mut i = Item::blank(ItemType::Service, name);
        i.income_account_id = Some(income);
        self.add_item(i)
    }

    pub fn add_tax_code(&mut self, taxable: bool) -> SalesTaxCodeId {
        let id = SalesTaxCodeId::new();
        self.tax_codes.insert(id, taxable);
        id
    }

    pub fn name_of(&self, id: AccountId) -> &str {
        self.accounts.get(&id).map_or("?", |(n, _)| n.as_str())
    }
}

impl PostingContext for MemoryBooks {
    fn account(&self, id: AccountId) -> Option<AccountInfo> {
        self.accounts.get(&id).map(|(_, i)| *i)
    }

    fn role_account(&self, role: AccountRole) -> Option<AccountId> {
        self.roles.get(&role).copied()
    }

    fn item(&self, id: ItemId) -> Option<Item> {
        self.items.get(&id).cloned()
    }

    fn entity_kind(&self, id: EntityId) -> Option<EntityKind> {
        self.entities.get(&id).map(|(_, k)| *k)
    }

    fn tax_code_is_taxable(&self, id: SalesTaxCodeId) -> bool {
        self.tax_codes.get(&id).copied().unwrap_or(false)
    }

    fn open_txn(&self, id: TxnId) -> Option<OpenTxnInfo> {
        self.open.get(&id).copied()
    }

    fn average_cost(&self, item: ItemId, _date: jiff::civil::Date, _excluding: TxnId) -> Decimal {
        self.costs.get(&item).copied().unwrap_or(Decimal::ZERO)
    }
}
