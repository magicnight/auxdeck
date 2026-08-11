//! hyte-daemon：常驻采集 + WebSocket RPC 服务（CLAUDE.md §3）。M1 范围：
//! sysinfo/nvml/lhm 三个 collector、1s 采样循环、WS 推送、单实例保护、
//! hyte-shell 守护进程。

mod collectors;
mod logging;
mod rpc;
mod single_instance;
mod supervisor;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyte_core::SystemSnapshot;
use tokio::sync::watch;
use tracing::{error, info};

use collectors::{Collector, LhmCollector, NvmlCollector, PartialMetrics, SysinfoCollector};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 汇总一轮所有 collector 的输出为完整快照。任一 collector 内部失败都已经
/// 在其自身实现里被吞掉（返回 None 字段），这里只负责合并与打包。
async fn sample_once(collectors: &mut [Box<dyn Collector>]) -> SystemSnapshot {
    let mut merged = PartialMetrics::default();
    for collector in collectors.iter_mut() {
        let partial = collector.collect().await;
        merged.merge(partial);
    }
    SystemSnapshot {
        ts_ms: now_ms(),
        cpu_usage: merged.cpu_usage.unwrap_or(0.0),
        cpu_temp: merged.cpu_temp,
        gpu_usage: merged.gpu_usage,
        gpu_temp: merged.gpu_temp,
    }
}

#[tokio::main]
async fn main() {
    let console = std::env::args().any(|a| a == "--console");
    let _log_guard = logging::init(console);

    info!(
        "hyte-daemon {} starting — rpc @ {}",
        env!("CARGO_PKG_VERSION"),
        hyte_core::RPC_ADDR
    );

    let _instance_lock = match single_instance::acquire() {
        single_instance::SingleInstance::AlreadyRunning => {
            info!("another hyte-daemon instance is already running, exiting");
            return;
        }
        single_instance::SingleInstance::Acquired(handle) => Some(handle),
        single_instance::SingleInstance::Unverified => None,
    };

    let mut collectors: Vec<Box<dyn Collector>> = vec![
        Box::new(SysinfoCollector::new()),
        Box::new(NvmlCollector::new()),
        Box::new(LhmCollector::new()),
    ];
    let names: Vec<&str> = collectors.iter().map(|c| c.name()).collect();
    info!("collectors enabled: {}", names.join(", "));

    // 启动前先采一轮真实数据，watch channel 不会以假数据起步。
    let initial_snapshot = sample_once(&mut collectors).await;
    let (tx, rx) = watch::channel(initial_snapshot);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);
        ticker.tick().await; // 消耗掉 interval 的立即触发，避免与上面的初始采样重复
        loop {
            ticker.tick().await;
            let snapshot = sample_once(&mut collectors).await;
            if tx.send(snapshot).is_err() {
                break; // 所有 receiver 已丢弃（rpc::serve 退出时才会发生）
            }
        }
    });

    tokio::spawn(supervisor::run());

    if let Err(err) = rpc::serve(rx).await {
        error!("WS server terminated: {err}");
    }
}
