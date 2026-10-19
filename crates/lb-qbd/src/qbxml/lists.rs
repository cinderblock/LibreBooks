//! List records: the chart of accounts, names, items and the small lists, plus
//! the company record and its preferences.

use lb_core::lists::{
    Class, PaymentMethod, PaymentMethodKind, SalesRep, Terms, TermsKind, TypeListKind,
};
use lb_core::{
    Account, AccountRole, AccountType, Address, CashFlowClass, CompanyInfo, Entity, EntityKind,
    Item, ItemComponent, ItemType, JobStatus,
};
use rust_decimal::Decimal;
use serde_json::Value;

use super::record::{Record, parse_bool};
use super::{ListImport, NormaliseError, Resolver, require};

/// Fields every list record carries that the sync engine reads itself (ids, edit
/// sequence, times) or that LibreBooks derives (full names, levels, balances).
const BOOKKEEPING: &[&str] = &[
    "ListID",
    "TimeCreated",
    "TimeModified",
    "EditSequence",
    "FullName",
    "Sublevel",
    "Balance",
    "TotalBalance",
];

fn unmapped(r: &Record<'_>, out: &mut Vec<String>) {
    for k in r.leftover_keys() {
        out.push(format!("{}.{k}", r.path));
    }
}

/// A QBD address (`Addr1`…`Addr5`, `City`, `State`, `PostalCode`, `Country`,
/// `Note`). The matching `…AddressBlock` is QBD's formatted copy and is ignored.
pub fn address(
    r: &mut Record<'_>,
    key: &str,
    out: &mut Vec<String>,
) -> Result<Address, NormaliseError> {
    let block = format!("{key}Block");
    r.ignore(&[block.as_str()]);
    let Some(mut a) = r.child(key)? else {
        return Ok(Address::default());
    };
    let mut lines = Vec::new();
    for k in ["Addr1", "Addr2", "Addr3", "Addr4", "Addr5"] {
        if let Some(l) = a.text(k) {
            lines.push(l);
        }
    }
    let addr = Address {
        lines,
        city: a.text("City"),
        state: a.text("State"),
        postal_code: a.text("PostalCode"),
        country: a.text("Country"),
        note: a.text("Note"),
    };
    unmapped(&a, out);
    Ok(addr)
}

/// QBD's `SpecialAccountType`, where it names a LibreBooks role unambiguously.
pub fn role_for_special(s: &str) -> Option<AccountRole> {
    Some(match s {
        "AccountsPayable" => AccountRole::AccountsPayable,
        "AccountsReceivable" => AccountRole::AccountsReceivable,
        "UndepositedFunds" => AccountRole::UndepositedFunds,
        "OpeningBalanceEquity" => AccountRole::OpeningBalanceEquity,
        "RetainedEarnings" => AccountRole::RetainedEarnings,
        "SalesTaxPayable" => AccountRole::SalesTaxPayable,
        "InventoryAssets" => AccountRole::InventoryAsset,
        "CostOfGoodsSold" => AccountRole::CostOfGoodsSold,
        "PayrollLiabilities" => AccountRole::PayrollLiabilities,
        "PayrollExpenses" => AccountRole::PayrollExpenses,
        "ReconciliationDifferences" => AccountRole::ReconciliationDiscrepancies,
        "Estimates" => AccountRole::Estimates,
        "PurchaseOrders" => AccountRole::PurchaseOrders,
        "SalesOrders" => AccountRole::SalesOrders,
        _ => return None,
    })
}

/// `AccountRet` → [`Account`]. The parent must already be imported (the sync
/// imports by `Sublevel`).
pub fn account(v: &Value, res: &mut dyn Resolver) -> Result<ListImport<Account>, NormaliseError> {
    let mut r = Record::new("Account", v)?;
    r.ignore(BOOKKEEPING);
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let name = r.required_text("Name")?;
    let type_text = r.required_text("AccountType")?;
    let account_type = AccountType::from_qbxml(&type_text).ok_or_else(|| {
        NormaliseError::Unsupported(format!(
            "account type {type_text:?} isn't one LibreBooks knows"
        ))
    })?;
    let parent_id = match r.reference("ParentRef") {
        Some(p) => Some(require("parent account", &p, res.account(&p))?),
        None => None,
    };
    let mut a = lb_store::accounts::new_account(&name, account_type);
    a.parent_id = parent_id;
    a.is_active = r.bool("IsActive")?.unwrap_or(true);
    a.number = r.text("AccountNumber");
    a.description = r.text("Desc");
    a.bank_account_number = r.text("BankNumber");
    if let Some(mut t) = r.child("TaxLineInfoRet")? {
        a.tax_line = t.text("TaxLineName");
        // The id is QBD's code for the same tax line; the name is what people see.
        t.ignore(&["TaxLineID"]);
        unmapped(&t, &mut out);
    }
    a.cash_flow_class = match r.text("CashFlowClassification").as_deref() {
        Some("Operating") => Some(CashFlowClass::Operating),
        Some("Investing") => Some(CashFlowClass::Investing),
        Some("Financing") => Some(CashFlowClass::Financing),
        Some("NotApplicable") => Some(CashFlowClass::NotApplicable),
        // "None": QBD hasn't classified it.
        _ => None,
    };
    if let Some(special) = r.text("SpecialAccountType") {
        a.role = role_for_special(&special);
        if a.role.is_none() {
            notes.push(format!(
                "QuickBooks marks it as its {special} account; LibreBooks has no such role"
            ));
        }
    }
    unmapped(&r, &mut out);
    Ok(ListImport {
        record: a,
        unmapped: out,
        notes,
    })
}

fn job_status(s: &str) -> Option<JobStatus> {
    Some(match s {
        "None" => JobStatus::None,
        "Pending" => JobStatus::Pending,
        "Awarded" => JobStatus::Awarded,
        "InProgress" => JobStatus::InProgress,
        "Closed" => JobStatus::Closed,
        "NotAwarded" => JobStatus::NotAwarded,
        _ => return None,
    })
}

/// Employee fields deliberately not imported: personal data LibreBooks has no
/// business holding for bookkeeping, and payroll setup it can't use yet.
const EMPLOYEE_PRIVATE: &[&str] = &[
    "SSN",
    "BirthDate",
    "Gender",
    "Ethnicity",
    "USCitizen",
    "USVeteran",
    "Disabled",
    "DisabilityDesc",
    "MilitaryStatus",
    "EmergencyContacts",
    "EmployeePayrollInfo",
    "OnFile",
    "WorkAuthExpireDate",
    "PagerPIN",
];

/// Customer, vendor, employee or other name → [`Entity`]. Customers' jobs carry a
/// `ParentRef` to their customer (imported first, by `Sublevel`).
pub fn entity(
    kind: EntityKind,
    v: &Value,
    res: &mut dyn Resolver,
) -> Result<ListImport<Entity>, NormaliseError> {
    let mut r = Record::new(kind.as_str(), v)?;
    r.ignore(BOOKKEEPING);
    let mut out = Vec::new();
    let name = r.required_text("Name")?;
    let mut e = Entity::blank(kind, &name);
    e.is_active = r.bool("IsActive")?.unwrap_or(true);
    if let Some(p) = r.reference("ParentRef") {
        e.parent_id = Some(require("parent customer", &p, res.entity(&p))?);
    }
    e.company_name = r.text("CompanyName");
    e.salutation = r.text("Salutation");
    e.first_name = r.text("FirstName");
    e.middle_name = r.text("MiddleName");
    e.last_name = r.text("LastName");
    e.job_title = r.text("JobTitle");
    e.phone = r.text("Phone");
    e.alt_phone = r.text("AltPhone");
    e.mobile = r.text("Mobile");
    e.fax = r.text("Fax");
    e.email = r.text("Email");
    e.cc_email = r.text("Cc");
    e.contact = r.text("Contact");
    e.alt_contact = r.text("AltContact");
    e.notes = r.text("Notes");
    e.account_number = r.text("AccountNumber");
    e.credit_limit = r.money("CreditLimit")?;
    let primary_address = match kind {
        EntityKind::Customer => "BillAddress",
        EntityKind::Vendor => "VendorAddress",
        EntityKind::Employee => "EmployeeAddress",
        EntityKind::OtherName => "OtherNameAddress",
    };
    e.bill_address = address(&mut r, primary_address, &mut out)?;
    e.ship_address = address(&mut r, "ShipAddress", &mut out)?;
    if let Some(t) = r.reference("TermsRef") {
        e.terms_id = Some(require("terms", &t, res.terms(&t))?);
    }
    if let Some(s) = r.reference("SalesRepRef") {
        e.sales_rep_id = Some(require("sales rep", &s, res.sales_rep(&s))?);
    }
    if let Some(s) = r.reference("SalesTaxCodeRef") {
        e.sales_tax_code_id = res.sales_tax_code(&s)?;
    }
    match kind {
        EntityKind::Customer => {
            if let Some(t) = r.reference("CustomerTypeRef") {
                e.type_id = res.type_entry(TypeListKind::CustomerType, &t)?;
            }
            e.tax_id = r.text("ResaleNumber");
            e.job_status = r.text("JobStatus").as_deref().and_then(job_status);
            e.job_start_date = r.date("JobStartDate")?;
            e.job_projected_end = r.date("JobProjectedEndDate")?;
            e.job_end_date = r.date("JobEndDate")?;
            e.job_description = r.text("JobDesc");
            if let Some(t) = r.reference("JobTypeRef") {
                e.job_type_id = res.type_entry(TypeListKind::JobType, &t)?;
            }
            // Card numbers are not something to copy around.
            r.ignore(&["CreditCardInfo"]);
        }
        EntityKind::Vendor => {
            if let Some(t) = r.reference("VendorTypeRef") {
                e.type_id = res.type_entry(TypeListKind::VendorType, &t)?;
            }
            e.print_as = r.text("NameOnCheck");
            e.tax_id = r.text("VendorTaxIdent");
            e.is_1099 = r.bool("IsVendorEligibleFor1099")?.unwrap_or(false);
            for a in r.references("PrefillAccountRef") {
                if e.default_expense_account_id.is_none() {
                    e.default_expense_account_id = res.account(&a)?;
                }
            }
        }
        EntityKind::Employee => {
            e.print_as = r.text("PrintAs");
            e.hired_date = r.date("HiredDate")?;
            e.released_date = r.date("ReleasedDate")?;
            r.ignore(EMPLOYEE_PRIVATE);
        }
        EntityKind::OtherName => {}
    }
    for mut c in r.children("AdditionalContactRef")? {
        let label = c.text("ContactName").unwrap_or_default();
        let value = c.text("ContactValue");
        let slot = match label.as_str() {
            "Mobile" => &mut e.mobile,
            "Main Phone" | "Work Phone" => &mut e.phone,
            "Alt. Phone" | "Alt. Mobile" | "Home Phone" => &mut e.alt_phone,
            "Fax" => &mut e.fax,
            "Main Email" => &mut e.email,
            "CC Email" => &mut e.cc_email,
            "Website" => &mut e.website,
            _ => {
                if let Some(v) = value {
                    e.custom_fields
                        .insert(format!("Contact: {label}"), Value::String(v));
                }
                continue;
            }
        };
        if slot.is_none() {
            *slot = value;
        }
    }
    for mut d in r.children("DataExtRet")? {
        if let (Some(k), Some(v)) = (d.text("DataExtName"), d.text("DataExtValue")) {
            e.custom_fields.insert(k, Value::String(v));
        }
        d.ignore(&["OwnerID", "DataExtType"]);
    }
    unmapped(&r, &mut out);
    Ok(ListImport {
        record: e,
        unmapped: out,
        notes: Vec::new(),
    })
}

/// The item type for a qbXML item element (`ItemServiceRet` → Service), for the
/// Web Connector transport, which keeps element names.
pub fn item_type_for_ret(element: &str) -> Option<ItemType> {
    Some(match element {
        "ItemServiceRet" => ItemType::Service,
        "ItemInventoryRet" => ItemType::Inventory,
        "ItemInventoryAssemblyRet" => ItemType::InventoryAssembly,
        "ItemNonInventoryRet" => ItemType::NonInventory,
        "ItemOtherChargeRet" => ItemType::OtherCharge,
        "ItemSubtotalRet" => ItemType::Subtotal,
        "ItemGroupRet" => ItemType::Group,
        "ItemDiscountRet" => ItemType::Discount,
        "ItemPaymentRet" => ItemType::Payment,
        "ItemSalesTaxRet" => ItemType::SalesTax,
        "ItemSalesTaxGroupRet" => ItemType::SalesTaxGroup,
        "ItemFixedAssetRet" => ItemType::FixedAsset,
        _ => return None,
    })
}

/// Tell an item's type from its shape. The bridge's generic `/items` drops the
/// qbXML element name, so: services and non-inventory parts come with a `hint`
/// from the typed lists; the rest are told apart by the fields only their type
/// has. Service, non-inventory and other-charge items share one shape
/// (`SalesOrPurchase` / `SalesAndPurchase`), so without a hint that shape is an
/// other charge.
pub fn classify_item(v: &Value, hint: Option<ItemType>) -> ItemType {
    if let Some(h) = hint {
        return h;
    }
    let has = |k: &str| v.get(k).is_some();
    if has("ItemInventoryAssemblyLine") || has("BuildPoint") {
        ItemType::InventoryAssembly
    } else if has("COGSAccountRef") || has("QuantityOnHand") {
        ItemType::Inventory
    } else if has("SalesOrPurchase") || has("SalesAndPurchase") {
        ItemType::OtherCharge
    } else if has("DiscountRate") || has("DiscountRatePercent") {
        ItemType::Discount
    } else if has("ItemGroupLine") || has("IsPrintItemsInGroup") {
        ItemType::Group
    } else if has("TaxRate") || has("TaxVendorRef") {
        ItemType::SalesTax
    } else if has("ItemSalesTaxRef") {
        ItemType::SalesTaxGroup
    } else if has("AcquiredAs") || has("FixedAssetSalesInfo") || has("AssetAccountRef") {
        ItemType::FixedAsset
    } else if has("DepositToAccountRef") || has("PaymentMethodRef") {
        ItemType::Payment
    } else {
        ItemType::Subtotal
    }
}

/// An item of a known type → [`Item`].
pub fn item(
    item_type: ItemType,
    v: &Value,
    res: &mut dyn Resolver,
) -> Result<ListImport<Item>, NormaliseError> {
    let mut r = Record::new(format!("Item({})", item_type.as_str()), v)?;
    r.ignore(BOOKKEEPING);
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let name = r.required_text("Name")?;
    let mut it = Item::blank(item_type, &name);
    it.is_active = r.bool("IsActive")?.unwrap_or(true);
    if let Some(p) = r.reference("ParentRef") {
        it.parent_id = Some(require("parent item", &p, res.item(&p))?);
    }
    it.unit_of_measure = r.reference("UnitOfMeasureSetRef").and_then(|u| u.full_name);
    if let Some(s) = r.reference("SalesTaxCodeRef") {
        it.sales_tax_code_id = res.sales_tax_code(&s)?;
    }
    if let Some(special) = r.text("SpecialItemType") {
        notes.push(format!("QuickBooks' own {special} item"));
    }
    let account = |res: &mut dyn Resolver, r: Option<super::QbRef>, what| match r {
        Some(a) => require(what, &a, res.account(&a)).map(Some),
        None => Ok(None),
    };
    match item_type {
        ItemType::Service | ItemType::NonInventory | ItemType::OtherCharge => {
            if let Some(mut s) = r.child("SalesOrPurchase")? {
                it.sales_description = s.text("Desc");
                if let Some(p) = s.decimal("PricePercent")? {
                    it.sales_price = Some(p);
                    it.is_percent = true;
                } else {
                    it.sales_price = s.decimal("Price")?;
                }
                it.income_account_id = account(res, s.reference("AccountRef"), "account")?;
                unmapped(&s, &mut out);
            } else if let Some(mut s) = r.child("SalesAndPurchase")? {
                it.has_purchase_info = true;
                it.sales_description = s.text("SalesDesc");
                it.sales_price = s.decimal("SalesPrice")?;
                it.income_account_id =
                    account(res, s.reference("IncomeAccountRef"), "income account")?;
                it.purchase_description = s.text("PurchaseDesc");
                it.purchase_cost = s.decimal("PurchaseCost")?;
                it.expense_account_id =
                    account(res, s.reference("ExpenseAccountRef"), "expense account")?;
                if let Some(v) = s.reference("PrefVendorRef") {
                    it.preferred_vendor_id = res.entity(&v)?;
                }
                unmapped(&s, &mut out);
            }
            it.sku = r.text("ManufacturerPartNumber");
        }
        ItemType::Inventory | ItemType::InventoryAssembly => {
            it.sku = r.text("ManufacturerPartNumber");
            it.sales_description = r.text("SalesDesc");
            it.sales_price = r.decimal("SalesPrice")?;
            it.income_account_id = account(res, r.reference("IncomeAccountRef"), "income account")?;
            it.has_purchase_info = true;
            it.purchase_description = r.text("PurchaseDesc");
            it.purchase_cost = r.decimal("PurchaseCost")?;
            it.expense_account_id = account(res, r.reference("COGSAccountRef"), "COGS account")?;
            if let Some(v) = r.reference("PrefVendorRef") {
                it.preferred_vendor_id = res.entity(&v)?;
            }
            it.asset_account_id = account(res, r.reference("AssetAccountRef"), "asset account")?;
            it.reorder_point = r.decimal("ReorderPoint")?;
            // On hand and average cost come from LibreBooks' own ledger.
            r.ignore(&[
                "QuantityOnHand",
                "AverageCost",
                "QuantityOnOrder",
                "QuantityOnSalesOrder",
            ]);
            for mut c in r.children("ItemInventoryAssemblyLine")? {
                let Some(m) = c.reference("ItemInventoryRef") else {
                    continue;
                };
                it.components.push(ItemComponent {
                    item_id: require("assembly component", &m, res.item(&m))?,
                    quantity: c.decimal("Quantity")?.unwrap_or(Decimal::ONE),
                });
            }
        }
        ItemType::Discount => {
            it.sales_description = r.text("ItemDesc");
            if let Some(p) = r.decimal("DiscountRatePercent")? {
                it.sales_price = Some(p);
                it.is_percent = true;
            } else {
                it.sales_price = r.decimal("DiscountRate")?;
            }
            it.income_account_id = account(res, r.reference("AccountRef"), "account")?;
        }
        ItemType::Subtotal => {
            it.sales_description = r.text("ItemDesc");
        }
        ItemType::Payment => {
            it.sales_description = r.text("ItemDesc");
            it.deposit_account_id =
                account(res, r.reference("DepositToAccountRef"), "deposit account")?;
        }
        ItemType::SalesTax => {
            it.sales_description = r.text("ItemDesc");
            it.tax_rate = r.decimal("TaxRate")?;
            if let Some(v) = r.reference("TaxVendorRef") {
                it.tax_agency_id = Some(require("tax agency", &v, res.entity(&v))?);
            }
        }
        ItemType::SalesTaxGroup => {
            it.sales_description = r.text("ItemDesc");
            for m in r.references("ItemSalesTaxRef") {
                it.components.push(ItemComponent {
                    item_id: require("sales tax item", &m, res.item(&m))?,
                    quantity: Decimal::ONE,
                });
            }
        }
        ItemType::Group => {
            it.sales_description = r.text("ItemDesc");
            it.print_components = r.bool("IsPrintItemsInGroup")?.unwrap_or(false);
            for mut c in r.children("ItemGroupLine")? {
                let Some(m) = c.reference("ItemRef") else {
                    continue;
                };
                it.components.push(ItemComponent {
                    item_id: require("group member", &m, res.item(&m))?,
                    quantity: c.decimal("Quantity")?.unwrap_or(Decimal::ONE),
                });
                c.ignore(&["UnitOfMeasure"]);
                unmapped(&c, &mut out);
            }
            if it.components.is_empty() {
                return Err(NormaliseError::Unsupported(
                    "a group item with no member items (QuickBooks keeps one for reimbursable expenses); LibreBooks needs members, and invoice lines that use it import as text lines".into(),
                ));
            }
        }
        ItemType::FixedAsset => {
            it.has_purchase_info = true;
            it.purchase_description = r.text("PurchaseDesc");
            it.purchase_cost = r.decimal("PurchaseCost")?;
            it.asset_account_id = account(res, r.reference("AssetAccountRef"), "asset account")?;
            it.sales_description = r.text("AssetDesc");
        }
    }
    unmapped(&r, &mut out);
    Ok(ListImport {
        record: it,
        unmapped: out,
        notes,
    })
}

pub fn class(v: &Value, res: &mut dyn Resolver) -> Result<ListImport<Class>, NormaliseError> {
    let mut r = Record::new("Class", v)?;
    r.ignore(BOOKKEEPING);
    let name = r.required_text("Name")?;
    let parent_id = match r.reference("ParentRef") {
        Some(p) => Some(require("parent class", &p, res.class(&p))?),
        None => None,
    };
    let c = Class {
        id: lb_core::ClassId::new(),
        full_name: name.clone(),
        name,
        parent_id,
        sublevel: 0,
        is_active: r.bool("IsActive")?.unwrap_or(true),
        edit_seq: 0,
    };
    let mut out = Vec::new();
    unmapped(&r, &mut out);
    Ok(ListImport {
        record: c,
        unmapped: out,
        notes: Vec::new(),
    })
}

/// Standard terms (`StdDueDays`…) or date-driven terms (`DayOfMonthDue`…). The
/// bridge flattens `StandardTermsRet` and `DateDrivenTermsRet` into one shape.
pub fn terms(v: &Value) -> Result<ListImport<Terms>, NormaliseError> {
    let mut r = Record::new("Terms", v)?;
    r.ignore(BOOKKEEPING);
    let name = r.required_text("Name")?;
    let discount_percent = r.decimal("DiscountPct")?.filter(|p| !p.is_zero());
    let to_u32 = |n: Option<i64>| n.and_then(|n| u32::try_from(n).ok());
    let to_u8 = |n: Option<i64>| n.and_then(|n| u8::try_from(n).ok());
    let kind = if r.has("DayOfMonthDue") {
        TermsKind::DateDriven {
            day_of_month: to_u8(r.integer("DayOfMonthDue")?).unwrap_or(1),
            cutoff_days: to_u8(r.integer("DueNextMonthDays")?).unwrap_or(0),
            discount_day_of_month: to_u8(r.integer("DiscountDayOfMonth")?).filter(|d| *d > 0),
            discount_percent,
        }
    } else {
        TermsKind::Standard {
            due_days: to_u32(r.integer("StdDueDays")?).unwrap_or(0),
            discount_days: to_u32(r.integer("StdDiscountDays")?).filter(|d| *d > 0),
            discount_percent,
        }
    };
    let t = Terms {
        id: lb_core::TermsId::new(),
        name,
        is_active: r.bool("IsActive")?.unwrap_or(true),
        kind,
        edit_seq: 0,
    };
    let mut out = Vec::new();
    unmapped(&r, &mut out);
    Ok(ListImport {
        record: t,
        unmapped: out,
        notes: Vec::new(),
    })
}

pub fn payment_method(v: &Value) -> Result<ListImport<PaymentMethod>, NormaliseError> {
    let mut r = Record::new("PaymentMethod", v)?;
    r.ignore(BOOKKEEPING);
    let name = r.required_text("Name")?;
    let kind = match r.text("PaymentMethodType").as_deref() {
        Some("Cash") => PaymentMethodKind::Cash,
        Some("Check") => PaymentMethodKind::Check,
        Some("AmericanExpress") => PaymentMethodKind::AmericanExpress,
        Some("Discover") => PaymentMethodKind::Discover,
        Some("MasterCard") => PaymentMethodKind::MasterCard,
        Some("Visa") => PaymentMethodKind::Visa,
        Some("DebitCard") => PaymentMethodKind::DebitCard,
        Some("GiftCard") => PaymentMethodKind::GiftCard,
        Some("ECheck") => PaymentMethodKind::ECheck,
        Some("OtherCreditCard") => PaymentMethodKind::OtherCreditCard,
        _ => PaymentMethodKind::Other,
    };
    let pm = PaymentMethod {
        id: lb_core::PaymentMethodId::new(),
        name,
        kind,
        is_active: r.bool("IsActive")?.unwrap_or(true),
        edit_seq: 0,
    };
    let mut out = Vec::new();
    unmapped(&r, &mut out);
    Ok(ListImport {
        record: pm,
        unmapped: out,
        notes: Vec::new(),
    })
}

pub fn sales_rep(
    v: &Value,
    res: &mut dyn Resolver,
) -> Result<ListImport<SalesRep>, NormaliseError> {
    let mut r = Record::new("SalesRep", v)?;
    r.ignore(BOOKKEEPING);
    let initials = r.required_text("Initial")?;
    let entity_id = match r.reference("SalesRepEntityRef") {
        Some(e) => Some(require("name", &e, res.entity(&e))?),
        None => None,
    };
    let s = SalesRep {
        id: lb_core::SalesRepId::new(),
        initials,
        entity_id,
        is_active: r.bool("IsActive")?.unwrap_or(true),
        edit_seq: 0,
    };
    let mut out = Vec::new();
    unmapped(&r, &mut out);
    Ok(ListImport {
        record: s,
        unmapped: out,
        notes: Vec::new(),
    })
}

fn month_number(s: &str) -> Option<i8> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(s))
        .map(|i| i as i8 + 1)
}

/// `CompanyRet` and `PreferencesRet` over what the books already hold. Fields QBD
/// doesn't send keep their current values.
pub fn company_info(
    company: &Value,
    preferences: Option<&Value>,
    current: &CompanyInfo,
) -> Result<CompanyInfo, NormaliseError> {
    let mut info = current.clone();
    let mut r = Record::new("Company", company)?;
    let mut ignored = Vec::new();
    if let Some(n) = r.text("CompanyName") {
        info.name = n;
    }
    info.legal_name = r.text("LegalCompanyName").or(info.legal_name);
    let a = address(&mut r, "Address", &mut ignored)?;
    if !a.is_empty() {
        info.address = a;
    }
    let a = address(&mut r, "LegalAddress", &mut ignored)?;
    if !a.is_empty() {
        info.legal_address = a;
    }
    info.phone = r.text("Phone").or(info.phone);
    info.email = r.text("Email").or(info.email);
    info.website = r.text("CompanyWebSite").or(info.website);
    info.ein = r.text("EIN").or(info.ein);
    if let Some(m) = r
        .text("FirstMonthFiscalYear")
        .as_deref()
        .and_then(month_number)
    {
        info.fiscal_year_start_month = m;
    }
    if let Some(m) = r
        .text("FirstMonthIncomeTaxYear")
        .as_deref()
        .and_then(month_number)
    {
        info.tax_year_start_month = m;
    }
    info.tax_form = r.text("TaxForm").or(info.tax_form);

    if let Some(p) = preferences {
        let get = |path: &[&str]| -> Option<bool> {
            let mut v = p;
            for k in path {
                v = v.get(*k)?;
            }
            v.as_str().and_then(parse_bool).or_else(|| v.as_bool())
        };
        let prefs = &mut info.preferences;
        if let Some(b) = get(&["AccountingPreferences", "IsUsingAccountNumbers"]) {
            prefs.use_account_numbers = b;
        }
        if let Some(b) = get(&["AccountingPreferences", "IsUsingClassTracking"]) {
            prefs.use_class_tracking = b;
        }
        if let Some(b) = get(&["AccountingPreferences", "IsAssigningJournalEntryNumbers"]) {
            prefs.auto_number_journal_entries = b;
        }
        if let Some(b) = get(&["JobsAndEstimatesPreferences", "IsUsingEstimates"]) {
            prefs.estimates_enabled = b;
        }
        if let Some(b) = get(&["JobsAndEstimatesPreferences", "IsUsingProgressInvoicing"]) {
            prefs.progress_invoicing = b;
        }
        if let Some(b) = get(&["MultiCurrencyPreferences", "IsMultiCurrencyOn"]) {
            prefs.multi_currency = b;
        }
        if let Some(b) = get(&["PurchasesAndVendorsPreferences", "IsUsingInventory"]) {
            prefs.inventory_enabled = b;
        }
        if let Some(b) = get(&["SalesAndCustomersPreferences", "IsAutoApplyingPayments"]) {
            prefs.auto_apply_payments = b;
        }
        if let Some(basis) = p
            .get("ReportsPreferences")
            .and_then(|r| r.get("SummaryReportBasis"))
            .and_then(Value::as_str)
        {
            prefs.reports_cash_basis = basis == "Cash";
        }
        // QBD only sends sales tax preferences when sales tax is turned on.
        match p.get("SalesTaxPreferences") {
            Some(st) => {
                prefs.sales_tax_enabled = true;
                if let Some(when) = st.get("PaySalesTax").and_then(Value::as_str) {
                    prefs.sales_tax_on_cash_basis = when == "UponReceiptOfPayment";
                }
            }
            None => prefs.sales_tax_enabled = false,
        }
    }
    Ok(info)
}

/// The sales tax codes QBD marks as the defaults for taxable and non-taxable
/// sales (from `SalesTaxPreferences`), to know which imported codes are taxable.
pub fn default_tax_codes(preferences: Option<&Value>) -> (Option<String>, Option<String>) {
    let st = preferences.and_then(|p| p.get("SalesTaxPreferences"));
    let name = |k: &str| {
        st.and_then(|s| s.get(k))
            .and_then(|r| r.get("FullName"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    (
        name("DefaultTaxableSalesTaxCodeRef"),
        name("DefaultNonTaxableSalesTaxCodeRef"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_items_by_shape() {
        let c = |v: Value| classify_item(&v, None);
        assert_eq!(
            c(json!({"SalesOrPurchase": {"Price": "1"}})),
            ItemType::OtherCharge
        );
        assert_eq!(
            classify_item(&json!({"SalesOrPurchase": {}}), Some(ItemType::Service)),
            ItemType::Service
        );
        assert_eq!(c(json!({"DiscountRatePercent": "10"})), ItemType::Discount);
        assert_eq!(c(json!({"DiscountRate": "5.00"})), ItemType::Discount);
        assert_eq!(
            c(json!({"ItemDesc": "x", "IsPrintItemsInGroup": "false"})),
            ItemType::Group
        );
        assert_eq!(
            c(json!({"ItemDesc": "x", "SpecialItemType": "ReimbursableExpenseSubtotal"})),
            ItemType::Subtotal
        );
        assert_eq!(
            c(json!({"Name": "x", "IsActive": "true"})),
            ItemType::Subtotal
        );
        assert_eq!(c(json!({"TaxRate": "8.25"})), ItemType::SalesTax);
        assert_eq!(
            c(json!({"ItemSalesTaxRef": [{"ListID": "1"}]})),
            ItemType::SalesTaxGroup
        );
        assert_eq!(
            c(json!({"COGSAccountRef": {}, "AssetAccountRef": {}})),
            ItemType::Inventory
        );
        assert_eq!(
            c(json!({"COGSAccountRef": {}, "ItemInventoryAssemblyLine": []})),
            ItemType::InventoryAssembly
        );
        assert_eq!(c(json!({"DepositToAccountRef": {}})), ItemType::Payment);
        assert_eq!(c(json!({"AcquiredAs": "New"})), ItemType::FixedAsset);
        assert_eq!(item_type_for_ret("ItemServiceRet"), Some(ItemType::Service));
    }

    #[test]
    fn months_by_name() {
        assert_eq!(month_number("January"), Some(1));
        assert_eq!(month_number("december"), Some(12));
        assert_eq!(month_number("Smarch"), None);
    }
}
