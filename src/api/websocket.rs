use axum::{
    extract::{State, ws::{WebSocket, WebSocketUpgrade, Message}},
    response::IntoResponse,
};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use futures::{SinkExt, StreamExt};
use crate::api::rest::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    
    // 업데이트 간격 (config에서 가져옴)
    let update_interval = 30; // 초
    let mut ticker = interval(Duration::from_secs(update_interval));

    loop {
        tokio::select! {
            // 주기적 업데이트 전송
            _ = ticker.tick() => {
                // 풀 데이터 수집
                let pools = state.collector.collect_all(&state.symbols).await;
                
                // 아비트라지 탐지
                let cex_prices = state.upbit.get_all_prices();
                let mut alerts = state.detector.detect_dex_dex(&pools);
                alerts.extend(state.detector.detect_dex_cex(&pools, &cex_prices));

                // 풀 업데이트 전송
                let pool_msg = serde_json::json!({
                    "type": "pool_update",
                    "count": pools.len(),
                    "data": pools,
                });
                if sender.send(Message::Text(pool_msg.to_string())).await.is_err() {
                    break;
                }

                // 아비트라지 알림 전송
                if !alerts.is_empty() {
                    let alert_msg = serde_json::json!({
                        "type": "arb_alert",
                        "count": alerts.len(),
                        "data": alerts,
                    });
                    if sender.send(Message::Text(alert_msg.to_string())).await.is_err() {
                        break;
                    }
                }
            }

            // 클라이언트 메시지 수신
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // 명령 처리 (예: 임계값 변경)
                        if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text) {
                            if cmd["type"] == "ping" {
                                let _ = sender.send(Message::Text(r#"{"type":"pong"}"#.to_string())).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
