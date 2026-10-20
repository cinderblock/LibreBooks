//! Live updates: Server-Sent Events of committed changes in one company, so an
//! open register shows another person's new check without a refresh.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use lb_core::CompanyId;
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::Principal;
use crate::error::ApiResult;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct EventsQuery {
    company: CompanyId,
}

pub async fn stream(
    State(s): State<AppState>,
    _p: Principal,
    Query(q): Query<EventsQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    // Opening the company now makes sure its changes are wired to the bus.
    s.companies.get(q.company)?;
    let company = q.company;
    let rx = s.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| match msg {
        Ok(change) if change.company == company => {
            let data = serde_json::json!({ "kind": change.kind, "id": change.id, "action": change.action });
            Some(Ok(Event::default().event("change").data(data.to_string())))
        }
        Ok(_) => None,
        // Too slow to keep up: tell the client to refetch everything.
        Err(_) => Some(Ok(Event::default().event("resync").data("{}"))),
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20))))
}
