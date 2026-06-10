//! A minimal Nostr relay client built directly on websockets so that we can
//! attribute every event to the specific relays that served it.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use super::event::Event;

/// Query one relay for the newest matching events.
///
/// Sends a single `REQ`, collects `EVENT`s until `EOSE` (or timeout), then
/// closes the subscription. Returns every event the relay served for the
/// filter; the caller picks the newest and merges attribution across relays.
pub async fn query_relay(
    relay: &str,
    authors: &[String],
    kind: u32,
    limit: u32,
    request_timeout: Duration,
) -> Vec<Event> {
    let filter = serde_json::json!({
        "authors": authors,
        "kinds": [kind],
        "limit": limit,
    });
    query_filter(relay, filter, request_timeout).await
}

/// Query one relay for the reverse edges of `target_hex`: kind-3 events that
/// carry the target in a `p` tag (`#p` filter). Each returned event's author is
/// a follower of the target. Best-effort — coverage depends on the relay.
pub async fn query_relay_followers(
    relay: &str,
    target_hex: &str,
    kind: u32,
    limit: u32,
    request_timeout: Duration,
) -> Vec<Event> {
    let filter = serde_json::json!({
        "kinds": [kind],
        "#p": [target_hex],
        "limit": limit,
    });
    query_filter(relay, filter, request_timeout).await
}

/// Run a single `REQ` with an arbitrary filter, with a timeout and error
/// logging, returning every event the relay served before `EOSE`.
async fn query_filter(
    relay: &str,
    filter: serde_json::Value,
    request_timeout: Duration,
) -> Vec<Event> {
    match timeout(request_timeout, query_relay_inner(relay, filter)).await {
        Ok(Ok(events)) => events,
        Ok(Err(e)) => {
            warn!(relay, error = %e, "relay query failed");
            Vec::new()
        }
        Err(_) => {
            debug!(relay, "relay query timed out");
            Vec::new()
        }
    }
}

async fn query_relay_inner(relay: &str, filter: serde_json::Value) -> anyhow::Result<Vec<Event>> {
    let (mut ws, _resp) = tokio_tungstenite::connect_async(relay).await?;

    let sub_id = "gns";
    let req = serde_json::json!(["REQ", sub_id, filter]);
    ws.send(Message::text(req.to_string())).await?;

    let mut events = Vec::new();
    while let Some(msg) = ws.next().await {
        let msg = msg?;
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => break,
            Message::Frame(_) => continue,
        };
        let value: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(arr) = value.as_array() else {
            continue;
        };
        match arr.first().and_then(|v| v.as_str()) {
            Some("EVENT") => {
                if let Some(ev_value) = arr.get(2) {
                    match serde_json::from_value::<Event>(ev_value.clone()) {
                        Ok(ev) => events.push(ev),
                        Err(e) => debug!(relay, error = %e, "could not parse event"),
                    }
                }
            }
            Some("EOSE") => break,
            Some("NOTICE") => debug!(relay, notice = ?arr.get(1), "relay notice"),
            _ => {}
        }
    }

    // Politely close the subscription; ignore errors on the way out.
    let close = serde_json::json!(["CLOSE", sub_id]);
    let _ = ws.send(Message::text(close.to_string())).await;
    let _ = ws.close(None).await;

    Ok(events)
}
