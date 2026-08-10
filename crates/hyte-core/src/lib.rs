//! HyteDeck 跨进程共享类型。daemon ↔ shell ↔ hostage 的所有数据结构定义在这里。
//! M1 起补充 serde 派生与 WebSocket 消息信封。

/// RPC 默认监听地址，仅本机回环（CLAUDE.md §4）。
pub const RPC_ADDR: &str = "127.0.0.1:9600";

/// daemon 推送的系统指标快照（M1 最小集）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemSnapshot {
    /// CPU 整体利用率，0.0–100.0。
    pub cpu_usage: f32,
    /// CPU 温度（℃）。LHM 不可用时为 None（CLAUDE.md §4）。
    pub cpu_temp: Option<f32>,
    /// GPU 利用率，0.0–100.0。
    pub gpu_usage: f32,
    /// GPU 温度（℃）。
    pub gpu_temp: f32,
}
