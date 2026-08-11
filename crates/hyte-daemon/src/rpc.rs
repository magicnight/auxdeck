//! WebSocket RPC 服务：监听 `hyte_core::RPC_ADDR`，支持多客户端；连接建立
//! 立即推最新快照，之后每次更新推 `Push::System`（CLAUDE.md 任务书 3）。

use futures_util::{SinkExt, StreamExt};
use hyte_core::{Push, SystemSnapshot, RPC_ADDR};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

/// 启动 WS 服务并持续接受连接，直到监听本身失败。
pub async fn serve(rx: watch::Receiver<SystemSnapshot>) -> std::io::Result<()> {
    let listener = TcpListener::bind(RPC_ADDR).await?;
    info!("WS listening on {RPC_ADDR}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let client_rx = rx.clone();
        tokio::spawn(async move {
            handle_client(stream, peer, client_rx).await;
        });
    }
}

async fn handle_client(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    mut rx: watch::Receiver<SystemSnapshot>,
) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(err) => {
            warn!("WS handshake with {peer} failed: {err}");
            return;
        }
    };
    debug!("client {peer} connected");

    let (mut write, mut read) = ws.split();

    let initial = *rx.borrow_and_update();
    if write
        .send(Message::Text(encode(&initial).into()))
        .await
        .is_err()
    {
        debug!("client {peer} disconnected before first push");
        return;
    }

    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let snapshot = *rx.borrow_and_update();
                if write.send(Message::Text(encode(&snapshot).into())).await.is_err() {
                    break;
                }
            }
            msg = read.next() => {
                match msg {
                    None | Some(Ok(Message::Close(_))) | Some(Err(_)) => break,
                    Some(Ok(_)) => {} // M1 只推不收，忽略入站消息
                }
            }
        }
    }
    debug!("client {peer} disconnected");
}

fn encode(snapshot: &SystemSnapshot) -> String {
    serde_json::to_string(&Push::System(*snapshot))
        .expect("Push::System serialization is infallible")
}
