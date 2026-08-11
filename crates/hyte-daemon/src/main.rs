//! hyte-daemon：常驻采集 + WebSocket RPC 服务（CLAUDE.md §3）。M2 范围：
//! config.toml 加载 + 热重载、天气/应用使用时长 collector、rpc 推送泛化为
//! 多路 Push（M1 已有 sysinfo/nvml/lhm 三个系统指标 collector、WS 推送、
//! 单实例保护、hyte-shell 守护进程）。

mod collectors;
mod config;
mod hub;
mod logging;
mod rpc;
mod single_instance;
mod supervisor;

use std::time::{SystemTime, UNIX_EPOCH};

use hyte_core::{Push, SystemSnapshot};
use tracing::{error, info};

use collectors::{Collector, LhmCollector, NvmlCollector, PartialMetrics, SysinfoCollector};
use hub::Hub;

/// Unix 毫秒时间戳，供各 collector 给推送打时间戳用。
pub(crate) fn now_ms() -> u64 {
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

// worker 线程压到 2：默认按 CPU 核数（本机 32）起线程，白白抬高常驻内存；
// daemon 负载只有 1s 采样与回环 WS，2 个 worker 足够（CLAUDE.md §1 <30MB 目标）。
#[tokio::main(worker_threads = 2)]
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

    let hub = Hub::new();

    let initial_config = config::load_or_init();
    info!(
        "config: system={}ms app_usage={}ms weather={}s weather.enabled={}",
        initial_config.sampling.system.as_millis(),
        initial_config.sampling.app_usage.as_millis(),
        initial_config.sampling.weather.as_secs(),
        initial_config.weather.enabled,
    );
    hub.publish(Push::Config(initial_config.shell.clone()));
    let cfg_rx = config::spawn_watcher(initial_config, hub.clone());

    let mut collectors: Vec<Box<dyn Collector>> = vec![
        Box::new(SysinfoCollector::new()),
        Box::new(NvmlCollector::new()),
        Box::new(LhmCollector::new()),
    ];
    let names: Vec<&str> = collectors.iter().map(|c| c.name()).collect();
    info!(
        "collectors enabled: {}, weather, app_usage",
        names.join(", ")
    );

    // 启动前先采一轮真实数据，客户端首次连接不会拿到假数据。
    let initial_snapshot = sample_once(&mut collectors).await;
    hub.publish(Push::System(initial_snapshot));

    {
        let hub = hub.clone();
        let mut cfg_rx = cfg_rx.clone();
        tokio::spawn(async move {
            loop {
                let interval = cfg_rx.borrow().sampling.system;
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {
                        let snapshot = sample_once(&mut collectors).await;
                        hub.publish(Push::System(snapshot));
                    }
                    changed = cfg_rx.changed() => {
                        if changed.is_err() {
                            break; // config watcher 线程已退出（进程关闭路径）
                        }
                        // 配置变了：重新读取 interval 再继续循环，本轮不采样。
                    }
                }
            }
        });
    }

    tokio::spawn(collectors::weather_collector::run(
        cfg_rx.clone(),
        hub.clone(),
    ));
    tokio::spawn(collectors::app_usage_collector::run(
        cfg_rx.clone(),
        hub.clone(),
    ));

    tokio::spawn(supervisor::run());

    if let Err(err) = rpc::serve(hub).await {
        error!("WS server terminated: {err}");
    }
}
