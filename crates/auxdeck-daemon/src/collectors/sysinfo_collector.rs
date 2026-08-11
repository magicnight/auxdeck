//! CPU 整体利用率采集。sysinfo 需要两次 refresh 之间存在时间间隔才能算出
//! CPU%，主循环 1s 节拍天然满足这个条件（CLAUDE.md 任务书 collector 1）。

use async_trait::async_trait;
use sysinfo::System;
use tracing::warn;

use super::{Collector, PartialMetrics};

pub struct SysinfoCollector {
    sys: System,
}

impl SysinfoCollector {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        Self { sys }
    }
}

impl Default for SysinfoCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Collector for SysinfoCollector {
    fn name(&self) -> &'static str {
        "sysinfo"
    }

    async fn collect(&mut self) -> PartialMetrics {
        self.sys.refresh_cpu_usage();
        let usage = self.sys.global_cpu_usage();
        if !usage.is_finite() {
            warn!("sysinfo returned non-finite CPU usage: {usage}");
            return PartialMetrics::default();
        }
        PartialMetrics {
            cpu_usage: Some(usage),
            ..Default::default()
        }
    }
}
