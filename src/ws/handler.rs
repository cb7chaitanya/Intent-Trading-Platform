use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::feed::{ClientMessage, ClientSession, WsFeed, WsFeedEvent};

const PING_INTERVAL_SECS: u64 = 30;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(feed): State<Arc<WsFeed>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, feed))
}

async fn handle_connection(socket: WebSocket, feed: Arc<WsFeed>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut session = ClientSession::new();
    let client_id = session.id;

    // Global events receiver
    let mut global_rx = feed.subscribe_global();

    // Per-market receivers — we'll manage them dynamically
    let (internal_tx, mut internal_rx) = tokio::sync::mpsc::channel::<String>(256);

    // Task: forward internal_rx + global_rx to ws_sender, plus periodic pings
    let sender_internal_tx = internal_tx.clone();
    let send_task = tokio::spawn(async move {
        let mut ping_interval =
            tokio::time::interval(tokio::time::Duration::from_secs(PING_INTERVAL_SECS));

        loop {
            tokio::select! {
                msg = internal_rx.recv() => {
                    match msg {
                        Some(text) => {
                            if ws_sender.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                msg = global_rx.recv() => {
                    if let Ok(text) = msg {
                        if ws_sender.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                }
                _ = ping_interval.tick() => {
                    if ws_sender.send(Message::Ping(vec![].into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Task: read from ws_receiver, handle subscribe/unsubscribe/ping
    let recv_feed = Arc::clone(&feed);
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            match msg {
                Message::Text(text) => {
                    let text_str: &str = &text;
                    if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(text_str) {
                        match client_msg {
                            ClientMessage::Subscribe { market_id } => {
                                if session.subscriptions.insert(market_id) {
                                    let mut rx = recv_feed.subscribe_market(&market_id).await;
                                    let tx = internal_tx.clone();
                                    // Spawn a forwarder for this market subscription
                                    tokio::spawn(async move {
                                        while let Ok(msg) = rx.recv().await {
                                            if tx.send(msg).await.is_err() {
                                                break;
                                            }
                                        }
                                    });
                                }
                                let ack = WsFeedEvent::Subscribed { market_id };
                                if let Ok(json) = serde_json::to_string(&ack) {
                                    let _ = internal_tx.send(json).await;
                                }
                            }
                            ClientMessage::Unsubscribe { market_id } => {
                                session.subscriptions.remove(&market_id);
                                let ack = WsFeedEvent::Unsubscribed { market_id };
                                if let Ok(json) = serde_json::to_string(&ack) {
                                    let _ = internal_tx.send(json).await;
                                }
                            }
                            ClientMessage::Ping => {
                                let pong = WsFeedEvent::Pong;
                                if let Ok(json) = serde_json::to_string(&pong) {
                                    let _ = internal_tx.send(json).await;
                                }
                            }
                        }
                    }
                }
                Message::Pong(_) => {} // keepalive response, ignore
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
}
