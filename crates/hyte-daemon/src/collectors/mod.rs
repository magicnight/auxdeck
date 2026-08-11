//! 统一采集器接口（CLAUDE.md §10）：collector 一律实现该 trait，可单独禁用；
//! 任一 collector 失败不得影响其余数据推送 —— 因此 `collect` 是 infallible 的，
//! 失败时对应字段返回 `None`，而不是向上传播错误中断主循环。

use async_trait::async_trait;

mod lhm_collector;
mod nvml_collector;
mod sysinfo_collector;

pub mod app_usage_collector;
mod app_usage_tracker;
pub mod weather_collector;

pub use lhm_collector::LhmCollector;
pub use nvml_collector::NvmlCollector;
pub use sysinfo_collector::SysinfoCollector;

/// 单次采样中，一个 collector 贡献的部分字段。未采集到的字段留 `None`，
/// 由主循环合并进最终 `SystemSnapshot`。
#[derive(Debug, Default, Clone, Copy)]
pub struct PartialMetrics {
    pub cpu_usage: Option<f32>,
    pub cpu_temp: Option<f32>,
    pub gpu_usage: Option<f32>,
    pub gpu_temp: Option<f32>,
}

impl PartialMetrics {
    /// 将 `other` 中已有的字段覆盖到 self 上，用于合并多个 collector 的输出。
    /// 各 collector 负责的字段互不重叠，这里的覆盖语义只是保证聚合方式统一。
    pub fn merge(&mut self, other: PartialMetrics) {
        if other.cpu_usage.is_some() {
            self.cpu_usage = other.cpu_usage;
        }
        if other.cpu_temp.is_some() {
            self.cpu_temp = other.cpu_temp;
        }
        if other.gpu_usage.is_some() {
            self.gpu_usage = other.gpu_usage;
        }
        if other.gpu_temp.is_some() {
            self.gpu_temp = other.gpu_temp;
        }
    }
}

/// 所有 collector 的统一接口。实现者必须自行吞掉内部错误并以 `None` 表达
/// "本次未采到"，绝不 panic、绝不让单个 collector 的问题影响其它 collector。
#[async_trait]
pub trait Collector: Send {
    /// 用于日志标识。
    fn name(&self) -> &'static str;

    /// 执行一次采样。
    async fn collect(&mut self) -> PartialMetrics;
}

/// 全进程共享的 HTTP 客户端：连接池与 TLS 上下文只保留一份（CLAUDE.md §1
/// daemon 内存目标）。超时由各调用方 per-request 指定，不在这里设默认值。
pub(crate) fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// 跟踪某个 collector 内部资源（NVML 句柄、LHM HTTP 连接等）的可用性，
/// 仅在状态发生变化时返回 Some，供调用方决定是否打日志 —— 避免采样循环
/// 每秒刷屏（CLAUDE.md 任务书对 nvml / lhm collector 的共同要求）。
#[derive(Debug, Default)]
pub struct AvailabilityState {
    ok: Option<bool>,
}

impl AvailabilityState {
    pub fn new() -> Self {
        Self { ok: None }
    }

    /// 上报本次采样的可用性；仅当与上次记录的状态不同（含首次上报）时返回
    /// `Some(是否变为可用)`，否则返回 `None` 表示无需记录日志。
    pub fn note(&mut self, ok: bool) -> Option<bool> {
        if self.ok == Some(ok) {
            None
        } else {
            self.ok = Some(ok);
            Some(ok)
        }
    }
}
