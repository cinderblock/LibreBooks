//! The same five routes for every list: accounts, names, items and the small lists.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use lb_core::lists::{Class, PaymentMethod, SalesRep, SalesTaxCode, Terms, TypeEntry};
use lb_core::{Account, CompanyId, Entity, EntityKind, Item, ItemType};
use lb_store::{Result as StoreResult, StoreError, Writer, accounts, entities, items, lists};
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::blocking;
use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Query parameters any list accepts; each resource uses what applies.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ListQuery {
    include_inactive: bool,
    kind: Option<String>,
    r#type: Option<String>,
}

/// A list the API serves.
trait Resource: Serialize + DeserializeOwned + Send + 'static {
    const NOUN: &'static str;
    fn list(c: &Connection, q: &ListQuery) -> StoreResult<Vec<Self>>;
    fn get(c: &Connection, id: &str) -> StoreResult<Self>;
    fn save(w: &mut Writer<'_>, rec: Self) -> StoreResult<Self>;
    fn delete(w: &mut Writer<'_>, id: &str, edit_seq: i64) -> StoreResult<()>;
    fn id_string(&self) -> String;
    fn edit_seq(&self) -> i64;
}

fn parse<T: std::str::FromStr>(id: &str, noun: &'static str) -> StoreResult<T> {
    id.parse().map_err(|_| StoreError::NotFound {
        kind: noun,
        id: id.to_owned(),
    })
}

impl Resource for Account {
    const NOUN: &'static str = "account";
    fn list(c: &Connection, q: &ListQuery) -> StoreResult<Vec<Self>> {
        accounts::list(c, q.include_inactive)
    }
    fn get(c: &Connection, id: &str) -> StoreResult<Self> {
        accounts::get(c, parse(id, Self::NOUN)?)
    }
    fn save(w: &mut Writer<'_>, rec: Self) -> StoreResult<Self> {
        accounts::save(w, rec)
    }
    fn delete(w: &mut Writer<'_>, id: &str, edit_seq: i64) -> StoreResult<()> {
        accounts::delete(w, parse(id, Self::NOUN)?, edit_seq)
    }
    fn id_string(&self) -> String {
        self.id.to_string()
    }
    fn edit_seq(&self) -> i64 {
        self.edit_seq
    }
}

impl Resource for Entity {
    const NOUN: &'static str = "name";
    fn list(c: &Connection, q: &ListQuery) -> StoreResult<Vec<Self>> {
        let kind = match q.kind.as_deref() {
            Some(k) => Some(
                EntityKind::parse(k)
                    .ok_or_else(|| StoreError::Invalid(format!("unknown kind {k}")))?,
            ),
            None => None,
        };
        entities::list(c, kind, q.include_inactive)
    }
    fn get(c: &Connection, id: &str) -> StoreResult<Self> {
        entities::get(c, parse(id, Self::NOUN)?)
    }
    fn save(w: &mut Writer<'_>, rec: Self) -> StoreResult<Self> {
        entities::save(w, rec)
    }
    fn delete(w: &mut Writer<'_>, id: &str, edit_seq: i64) -> StoreResult<()> {
        entities::delete(w, parse(id, Self::NOUN)?, edit_seq)
    }
    fn id_string(&self) -> String {
        self.id.to_string()
    }
    fn edit_seq(&self) -> i64 {
        self.edit_seq
    }
}

impl Resource for Item {
    const NOUN: &'static str = "item";
    fn list(c: &Connection, q: &ListQuery) -> StoreResult<Vec<Self>> {
        let t = match q.r#type.as_deref() {
            Some(t) => Some(
                ItemType::parse(t)
                    .ok_or_else(|| StoreError::Invalid(format!("unknown item type {t}")))?,
            ),
            None => None,
        };
        items::list(c, t, q.include_inactive)
    }
    fn get(c: &Connection, id: &str) -> StoreResult<Self> {
        items::get(c, parse(id, Self::NOUN)?)
    }
    fn save(w: &mut Writer<'_>, rec: Self) -> StoreResult<Self> {
        items::save(w, rec)
    }
    fn delete(w: &mut Writer<'_>, id: &str, edit_seq: i64) -> StoreResult<()> {
        items::delete(w, parse(id, Self::NOUN)?, edit_seq)
    }
    fn id_string(&self) -> String {
        self.id.to_string()
    }
    fn edit_seq(&self) -> i64 {
        self.edit_seq
    }
}

macro_rules! small_list {
    ($t:ty) => {
        impl Resource for $t {
            const NOUN: &'static str = <$t as lists::ListRecord>::NOUN;
            fn list(c: &Connection, q: &ListQuery) -> StoreResult<Vec<Self>> {
                lists::list::<$t>(c, q.include_inactive)
            }
            fn get(c: &Connection, id: &str) -> StoreResult<Self> {
                lists::get::<$t>(c, id)
            }
            fn save(w: &mut Writer<'_>, rec: Self) -> StoreResult<Self> {
                lists::save(w, rec)
            }
            fn delete(w: &mut Writer<'_>, id: &str, edit_seq: i64) -> StoreResult<()> {
                lists::delete::<$t>(w, id, edit_seq)
            }
            fn id_string(&self) -> String {
                lists::ListRecord::id_string(self)
            }
            fn edit_seq(&self) -> i64 {
                lists::ListRecord::edit_seq(self)
            }
        }
    };
}

small_list!(Class);
small_list!(Terms);
small_list!(PaymentMethod);
small_list!(SalesTaxCode);
small_list!(SalesRep);
small_list!(TypeEntry);

async fn list_h<R: Resource>(
    State(s): State<AppState>,
    _p: Principal,
    Path(company): Path<CompanyId>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<R>>> {
    let books = s.companies.get(company)?;
    Ok(Json(
        blocking(move || books.read(|c| R::list(c, &q))).await?,
    ))
}

async fn get_h<R: Resource>(
    State(s): State<AppState>,
    _p: Principal,
    Path((company, id)): Path<(CompanyId, String)>,
) -> ApiResult<Json<R>> {
    let books = s.companies.get(company)?;
    Ok(Json(
        blocking(move || books.read(|c| R::get(c, &id))).await?,
    ))
}

async fn create_h<R: Resource>(
    State(s): State<AppState>,
    p: Principal,
    Path(company): Path<CompanyId>,
    Json(rec): Json<R>,
) -> ApiResult<(StatusCode, Json<R>)> {
    p.require(Role::Bookkeeper)?;
    if rec.edit_seq() != 0 {
        return Err(ApiError::invalid("a new record has edit_seq 0"));
    }
    let books = s.companies.get(company)?;
    let actor = p.actor();
    let saved = blocking(move || books.write(&actor, |w| R::save(w, rec))).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn update_h<R: Resource>(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, String)>,
    Json(rec): Json<R>,
) -> ApiResult<Json<R>> {
    p.require(Role::Bookkeeper)?;
    if rec.id_string() != id {
        return Err(ApiError::invalid("the id in the path and the body differ"));
    }
    if rec.edit_seq() == 0 {
        return Err(ApiError::invalid(
            "updating needs the edit_seq it was read with",
        ));
    }
    let books = s.companies.get(company)?;
    let actor = p.actor();
    Ok(Json(
        blocking(move || books.write(&actor, |w| R::save(w, rec))).await?,
    ))
}

#[derive(Deserialize)]
struct DeleteQuery {
    edit_seq: i64,
}

async fn delete_h<R: Resource>(
    State(s): State<AppState>,
    p: Principal,
    Path((company, id)): Path<(CompanyId, String)>,
    Query(q): Query<DeleteQuery>,
) -> ApiResult<StatusCode> {
    p.require(Role::Bookkeeper)?;
    let books = s.companies.get(company)?;
    let actor = p.actor();
    blocking(move || books.write(&actor, |w| R::delete(w, &id, q.edit_seq))).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn resource<R: Resource>(router: Router<AppState>, path: &str) -> Router<AppState> {
    router
        .route(path, get(list_h::<R>).post(create_h::<R>))
        .route(
            &format!("{path}/{{id}}"),
            get(get_h::<R>).put(update_h::<R>).delete(delete_h::<R>),
        )
}

pub fn routes() -> Router<AppState> {
    let r = Router::new();
    let r = resource::<Account>(r, "/accounts");
    let r = resource::<Entity>(r, "/entities");
    let r = resource::<Item>(r, "/items");
    let r = resource::<Class>(r, "/classes");
    let r = resource::<Terms>(r, "/terms");
    let r = resource::<PaymentMethod>(r, "/payment-methods");
    let r = resource::<SalesTaxCode>(r, "/sales-tax-codes");
    let r = resource::<SalesRep>(r, "/sales-reps");
    resource::<TypeEntry>(r, "/types")
}
