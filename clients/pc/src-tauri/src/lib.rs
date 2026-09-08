//! atlas-bot PC Tauri shell — I2.1 Bearer WS via rust (browser cannot set Authorization).

#![forbid(unsafe_code)]

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::Request, Message},
};

struct WsState {
    /// Outbound text frames from frontend → hub.
    write_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
}

#[derive(Debug, Serialize)]
struct ConnectResult {
    ok: bool,
    detail: String,
}

/// Open a Hub WebSocket with `Authorization: Bearer <token>` on the upgrade.
///
/// Browser `WebSocket` cannot set custom headers; production PC must use this
/// command when Hub is in `static` / `oidc` mode.
#[tauri::command]
async fn connect_ws(
    url: String,
    authorization: Option<String>,
    state: State<'_, WsState>,
) -> Result<ConnectResult, String> {
    // Tear down any previous writer.
    {
        let mut slot = state.write_tx.lock().await;
        *slot = None;
    }

    let req = build_ws_request(&url, authorization.as_deref())?;
    let (ws, _) = connect_async(req)
        .await
        .map_err(|e| format!("ws connect: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    {
        let mut slot = state.write_tx.lock().await;
        *slot = Some(tx);
    }

    // Writer task: frontend → hub
    tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Reader task drains until close (hello/RPC handled by a fuller bridge later;
    // this command proves Bearer upgrade works; frontend may still use browser WS
    // under `dev`).
    tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    Ok(ConnectResult {
        ok: true,
        detail: if authorization.is_some() {
            "connected with Authorization Bearer".into()
        } else {
            "connected without Authorization".into()
        },
    })
}

/// Send a text frame on the Tauri-managed WS (optional companion to connect_ws).
#[tauri::command]
async fn ws_send(text: String, state: State<'_, WsState>) -> Result<(), String> {
    let slot = state.write_tx.lock().await;
    let tx = slot
        .as_ref()
        .ok_or_else(|| "not connected via connect_ws".to_string())?;
    tx.send(text).map_err(|e| e.to_string())
}

fn build_ws_request(
    url: &str,
    bearer: Option<&str>,
) -> Result<Request<()>, String> {
    let mut req = url
        .into_client_request()
        .map_err(|e| format!("bad url: {e}"))?;
    if let Some(token) = bearer {
        if !token.is_empty() {
            let val = if token.starts_with("Bearer ") || token.starts_with("bearer ") {
                token.to_string()
            } else {
                format!("Bearer {token}")
            };
            req.headers_mut().insert(
                http::header::AUTHORIZATION,
                val.parse().map_err(|e| format!("authorization header: {e}"))?,
            );
        }
    }
    Ok(req)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(WsState {
            write_tx: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![connect_ws, ws_send])
        .run(tauri::generate_context!())
        .expect("error while running atlas-bot-pc");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_sets_authorization() {
        let req = build_ws_request("ws://127.0.0.1:7700/ws", Some("tok.tok.tok")).unwrap();
        let h = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(h, "Bearer tok.tok.tok");
    }
}
