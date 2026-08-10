//! hyte-daemon：常驻采集 + WebSocket RPC 服务（CLAUDE.md §3）。
//! 当前为 workspace 脚手架入口，M1 实现 collectors 与 RPC。

fn main() {
    println!(
        "hyte-daemon {} — rpc @ {}",
        env!("CARGO_PKG_VERSION"),
        hyte_core::RPC_ADDR
    );
}
