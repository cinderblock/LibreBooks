//! Items: what appears on the "Items" tab of a form.
//!
//! An item decides which accounts a line posts to. QBD's item types are kept one
//! for one, because each posts differently: an inventory part moves its cost from
//! the asset account to cost of goods sold when sold; a discount is a negative
//! line; a subtotal posts nothing; a group expands into its members.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{AccountId, EntityId, ItemId, SalesTaxCodeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ItemType {
    Service,
    Inventory,
    InventoryAssembly,
    NonInventory,
    OtherCharge,
    Subtotal,
    Group,
    Discount,
    Payment,
    SalesTax,
    SalesTaxGroup,
    FixedAsset,
}

impl ItemType {
    pub const ALL: [ItemType; 12] = [
        ItemType::Service,
        ItemType::Inventory,
        ItemType::InventoryAssembly,
        ItemType::NonInventory,
        ItemType::OtherCharge,
        ItemType::Subtotal,
        ItemType::Group,
        ItemType::Discount,
        ItemType::Payment,
        ItemType::SalesTax,
        ItemType::SalesTaxGroup,
        ItemType::FixedAsset,
    ];

    /// Items whose quantity on hand LibreBooks tracks.
    pub const fn is_inventory(self) -> bool {
        matches!(self, ItemType::Inventory | ItemType::InventoryAssembly)
    }

    pub const fn label(self) -> &'static str {
        match self {
            ItemType::Service => "Service",
            ItemType::Inventory => "Inventory Part",
            ItemType::InventoryAssembly => "Inventory Assembly",
            ItemType::NonInventory => "Non-inventory Part",
            ItemType::OtherCharge => "Other Charge",
            ItemType::Subtotal => "Subtotal",
            ItemType::Group => "Group",
            ItemType::Discount => "Discount",
            ItemType::Payment => "Payment",
            ItemType::SalesTax => "Sales Tax Item",
            ItemType::SalesTaxGroup => "Sales Tax Group",
            ItemType::FixedAsset => "Fixed Asset",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ItemType::Service => "Service",
            ItemType::Inventory => "Inventory",
            ItemType::InventoryAssembly => "InventoryAssembly",
            ItemType::NonInventory => "NonInventory",
            ItemType::OtherCharge => "OtherCharge",
            ItemType::Subtotal => "Subtotal",
            ItemType::Group => "Group",
            ItemType::Discount => "Discount",
            ItemType::Payment => "Payment",
            ItemType::SalesTax => "SalesTax",
            ItemType::SalesTaxGroup => "SalesTaxGroup",
            ItemType::FixedAsset => "FixedAsset",
        }
    }

    pub fn parse(s: &str) -> Option<ItemType> {
        ItemType::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// One member of a group or assembly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ItemComponent {
    pub item_id: ItemId,
    #[ts(type = "string")]
    pub quantity: Decimal,
}

/// An item. Which fields mean something depends on [`ItemType`]; see each field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Item {
    pub id: ItemId,
    pub item_type: ItemType,
    pub name: String,
    /// `Rental Income:Cube Space`.
    pub full_name: String,
    pub parent_id: Option<ItemId>,
    pub sublevel: u32,
    pub is_active: bool,
    pub sku: Option<String>,
    pub unit_of_measure: Option<String>,

    /// What a sale says, and its price (or, for discounts and charges given as a
    /// percent, the percent: see `is_percent`).
    pub sales_description: Option<String>,
    #[ts(type = "string | null")]
    pub sales_price: Option<Decimal>,
    /// Discount / other charge / sales tax given as a percent rather than an amount.
    pub is_percent: bool,
    /// Where a sale posts (Service, `NonInventory`, `OtherCharge`, Discount, Inventory…).
    pub income_account_id: Option<AccountId>,

    /// "This item is used in assemblies or is purchased for a specific customer:job"
    /// (QBD's two-sided items), and the purchase side's fields.
    pub has_purchase_info: bool,
    pub purchase_description: Option<String>,
    #[ts(type = "string | null")]
    pub purchase_cost: Option<Decimal>,
    /// Where a purchase posts (expense or COGS for non-inventory/service,
    /// COGS for inventory sales).
    pub expense_account_id: Option<AccountId>,
    pub preferred_vendor_id: Option<EntityId>,

    /// Inventory: asset account, reorder point, on-hand (maintained by the store).
    pub asset_account_id: Option<AccountId>,
    #[ts(type = "string | null")]
    pub reorder_point: Option<Decimal>,
    #[ts(type = "string")]
    pub quantity_on_hand: Decimal,
    #[ts(type = "string")]
    pub average_cost: Decimal,

    /// Sales tax items: the rate (percent) and the agency (a vendor) it is owed to.
    #[ts(type = "string | null")]
    pub tax_rate: Option<Decimal>,
    pub tax_agency_id: Option<EntityId>,
    /// Is a sale of this item taxable by default.
    pub sales_tax_code_id: Option<SalesTaxCodeId>,

    /// Groups (and sales tax groups, and assemblies): members and quantities.
    pub components: Vec<ItemComponent>,
    /// Groups: print the members on the form, or just the group.
    pub print_components: bool,

    /// Payment items: deposit to Undeposited Funds or this account.
    pub deposit_account_id: Option<AccountId>,
    #[ts(type = "number")]
    pub edit_seq: i64,
}

impl Item {
    pub fn blank(item_type: ItemType, name: &str) -> Item {
        Item {
            id: ItemId::new(),
            item_type,
            name: name.to_owned(),
            full_name: name.to_owned(),
            parent_id: None,
            sublevel: 0,
            is_active: true,
            sku: None,
            unit_of_measure: None,
            sales_description: None,
            sales_price: None,
            is_percent: false,
            income_account_id: None,
            has_purchase_info: false,
            purchase_description: None,
            purchase_cost: None,
            expense_account_id: None,
            preferred_vendor_id: None,
            asset_account_id: None,
            reorder_point: None,
            quantity_on_hand: Decimal::ZERO,
            average_cost: Decimal::ZERO,
            tax_rate: None,
            tax_agency_id: None,
            sales_tax_code_id: None,
            components: Vec::new(),
            print_components: false,
            deposit_account_id: None,
            edit_seq: 0,
        }
    }
}
