//! Push 分发中心：缓存每类推送的最新值 + 用 broadcast 扇出更新给所有已连接
//! 客户端。新连接时先按固定顺序推 Config/System/Weather/AppUsage 的最新值
//! （有则推），此后到达的更新通过 broadcast 实时转发给所有客户端
//! （CLAUDE.md 任务书 4）。是同步、非阻塞的：既能被 tokio 任务调用，也能被
//! config.rs 里那条监听文件变化的普通 std::thread 直接调用。

use std::sync::{Arc, RwLock};

use hyte_core::{AppUsageSnapshot, Push, ShellConfig, SystemSnapshot, WeatherSnapshot};
use tokio::sync::broadcast;

/// 广播通道容量：客户端短暂落后时的缓冲深度，超出后旧消息被丢弃
/// （客户端会收到 `RecvError::Lagged`，rpc.rs 只记日志，不需要重连）。
const BROADCAST_CAPACITY: usize = 32;

#[derive(Default)]
struct LatestPushes {
    config: Option<ShellConfig>,
    system: Option<SystemSnapshot>,
    weather: Option<WeatherSnapshot>,
    app_usage: Option<AppUsageSnapshot>,
}

/// 廉价可克隆的句柄：内部用 `Arc` 共享同一份最新状态缓存与广播通道。
#[derive(Clone)]
pub struct Hub {
    latest: Arc<RwLock<LatestPushes>>,
    tx: broadcast::Sender<Push>,
}

impl Hub {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            latest: Arc::new(RwLock::new(LatestPushes::default())),
            tx,
        }
    }

    /// 更新最新缓存并广播给所有当前订阅者。没有订阅者时 `send` 会返回
    /// `Err`，属预期（此时只需要缓存更新，等下一个连接进来即可拿到最新值），
    /// 因此这里忽略返回值。
    pub fn publish(&self, push: Push) {
        {
            let mut latest = self
                .latest
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match &push {
                Push::Config(c) => latest.config = Some(c.clone()),
                Push::System(s) => latest.system = Some(*s),
                Push::Weather(w) => latest.weather = Some(w.clone()),
                Push::AppUsage(a) => latest.app_usage = Some(a.clone()),
            }
        }
        let _ = self.tx.send(push);
    }

    /// 新客户端连接时应立即推送的存量数据，固定顺序：Config → System →
    /// Weather → AppUsage，缺失的类型（尚未产生首个快照）直接跳过。
    pub fn snapshot(&self) -> Vec<Push> {
        let latest = self
            .latest
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut out = Vec::with_capacity(4);
        if let Some(c) = &latest.config {
            out.push(Push::Config(c.clone()));
        }
        if let Some(s) = &latest.system {
            out.push(Push::System(*s));
        }
        if let Some(w) = &latest.weather {
            out.push(Push::Weather(w.clone()));
        }
        if let Some(a) = &latest.app_usage {
            out.push(Push::AppUsage(a.clone()));
        }
        out
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Push> {
        self.tx.subscribe()
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}
