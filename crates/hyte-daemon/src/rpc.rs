//! WebSocket RPC 服务：监听 `hyte_core::RPC_ADDR`，支持多客户端；连接建立
//! 时立即按序推 Config/System/Weather/AppUsage 的最新快照（有则推），此后
//! 每次有新数据到达时广播给所有客户端（CLAUDE.md 任务书 4）。所有状态缓存
//! 与扇出都在 `hub::Hub` 里，这里只负责 WS 协议本身。

use futures_util::{SinkExt, StreamExt};
use hyte_core::{Push, RPC_ADDR};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::hub::Hub;

/// 启动 WS 服务并持续接受连接，直到监听本身失败。
pub async fn serve(hub: Hub) -> std::io::Result<()> {
    let listener = TcpListener::bind(RPC_ADDR).await?;
    info!("WS listening on {RPC_ADDR}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let client_hub = hub.clone();
        tokio::spawn(async move {
            handle_client(stream, peer, client_hub).await;
        });
    }
}

async fn handle_client(stream: TcpStream, peer: std::net::SocketAddr, hub: Hub) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(err) => {
            warn!("WS handshake with {peer} failed: {err}");
            return;
        }
    };
    debug!("client {peer} connected");

    let (mut write, mut read) = ws.split();
    let mut updates = hub.subscribe();

    for push in hub.snapshot() {
        if write
            .send(Message::Text(encode(&push).into()))
            .await
            .is_err()
        {
            debug!("client {peer} disconnected before initial push completed");
            return;
        }
    }

    loop {
        tokio::select! {
            update = updates.recv() => {
                match update {
                    Ok(push) => {
                        if write.send(Message::Text(encode(&push).into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("client {peer} lagged, skipped {skipped} pushes");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = read.next() => {
                match msg {
                    None | Some(Ok(Message::Close(_))) | Some(Err(_)) => break,
                    Some(Ok(_)) => {} // M2 仍只推不收，忽略入站消息
                }
            }
        }
    }
    debug!("client {peer} disconnected");
}

fn encode(push: &Push) -> String {
    serde_json::to_string(push).expect("Push serialization is infallible")
}
