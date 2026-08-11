//! GPU 利用率与温度采集（CLAUDE.md 任务书 collector 2）。`Nvml::init` 失败时
//! 两个字段返回 `None`，仅在初始化失败或后续查询可用性变化时打 warn，避免刷屏。

use async_trait::async_trait;
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::Nvml;
use tracing::{info, warn};

use super::{AvailabilityState, Collector, PartialMetrics};

pub struct NvmlCollector {
    nvml: Option<Nvml>,
    availability: AvailabilityState,
}

impl NvmlCollector {
    pub fn new() -> Self {
        let nvml = match Nvml::init() {
            Ok(nvml) => Some(nvml),
            Err(err) => {
                warn!("NVML init failed, GPU metrics disabled: {err}");
                None
            }
        };
        Self {
            nvml,
            availability: AvailabilityState::new(),
        }
    }
}

impl Default for NvmlCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Collector for NvmlCollector {
    fn name(&self) -> &'static str {
        "nvml"
    }

    async fn collect(&mut self) -> PartialMetrics {
        let Some(nvml) = self.nvml.as_ref() else {
            return PartialMetrics::default();
        };

        let sample = nvml.device_by_index(0).map(|device| {
            let usage = device.utilization_rates().ok().map(|u| u.gpu as f32);
            let temp = device
                .temperature(TemperatureSensor::Gpu)
                .ok()
                .map(|t| t as f32);
            (usage, temp)
        });

        let (usage, temp) = match sample {
            Ok(pair) => pair,
            Err(err) => {
                if self.availability.note(false).is_some() {
                    warn!("NVML query failed: {err}");
                }
                return PartialMetrics::default();
            }
        };

        match self.availability.note(usage.is_some() || temp.is_some()) {
            Some(true) => info!("NVML metrics available"),
            Some(false) => warn!("NVML query succeeded but returned no usable data this tick"),
            None => {}
        }

        PartialMetrics {
            gpu_usage: usage,
            gpu_temp: temp,
            ..Default::default()
        }
    }
}
