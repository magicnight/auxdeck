//! CPU 温度采集，轮询 LibreHardwareMonitor 的 HTTP server
//! （CLAUDE.md 任务书 collector 3）。LHM 返回嵌套 Children 树；按优先级
//! 依次查找 Text 命中的第一个节点，解析 Value 前缀浮点数。连接失败或解析
//! 失败时 `cpu_temp` 为 `None`；仅在可用性状态变化时打日志，避免刷屏。

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tracing::{info, warn};

use super::{AvailabilityState, Collector, PartialMetrics};

const LHM_URL: &str = "http://localhost:8085/data.json";
/// 按优先级依次尝试的传感器名；命中第一个即停止查找。
const TEMP_CANDIDATES: [&str; 3] = ["Core (Tctl/Tdie)", "CPU Package", "Package"];

#[derive(Debug, Default, Deserialize)]
struct LhmNode {
    #[serde(default, rename = "Text")]
    text: String,
    #[serde(default, rename = "Value")]
    value: String,
    #[serde(default, rename = "Children")]
    children: Vec<LhmNode>,
}

fn find_by_text<'a>(node: &'a LhmNode, target: &str) -> Option<&'a LhmNode> {
    if node.text == target {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_by_text(child, target))
}

/// 解析形如 "65.5 °C" 的字符串，取前缀浮点数部分。
fn parse_leading_f32(s: &str) -> Option<f32> {
    let s = s.trim();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(s.len());
    s[..end].parse::<f32>().ok()
}

fn extract_cpu_temp(root: &LhmNode) -> Option<f32> {
    TEMP_CANDIDATES
        .iter()
        .find_map(|name| find_by_text(root, name).and_then(|node| parse_leading_f32(&node.value)))
}

pub struct LhmCollector {
    client: reqwest::Client,
    availability: AvailabilityState,
}

impl LhmCollector {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(800))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            availability: AvailabilityState::new(),
        }
    }
}

impl Default for LhmCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Collector for LhmCollector {
    fn name(&self) -> &'static str {
        "lhm"
    }

    async fn collect(&mut self) -> PartialMetrics {
        let result: Result<LhmNode, reqwest::Error> = async {
            let root = self
                .client
                .get(LHM_URL)
                .send()
                .await?
                .error_for_status()?
                .json::<LhmNode>()
                .await?;
            Ok(root)
        }
        .await;

        let root = match result {
            Ok(root) => root,
            Err(err) => {
                if self.availability.note(false).is_some() {
                    warn!("LHM unavailable: {err}");
                }
                return PartialMetrics::default();
            }
        };

        let temp = extract_cpu_temp(&root);
        match self.availability.note(temp.is_some()) {
            Some(true) => info!("LHM CPU temperature available"),
            Some(false) => warn!("LHM reachable but no matching temperature node found"),
            None => {}
        }

        PartialMetrics {
            cpu_temp: temp,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_core_tctl_nested() {
        let json = r#"{
            "Text": "Computer",
            "Value": "",
            "Children": [
                {
                    "Text": "MYPC",
                    "Value": "",
                    "Children": [
                        {
                            "Text": "AMD Ryzen 9",
                            "Value": "",
                            "Children": [
                                { "Text": "Voltage", "Value": "1.2 V", "Children": [] },
                                { "Text": "Core (Tctl/Tdie)", "Value": "65.5 °C", "Children": [] }
                            ]
                        }
                    ]
                }
            ]
        }"#;
        let root: LhmNode = serde_json::from_str(json).unwrap();
        assert_eq!(extract_cpu_temp(&root), Some(65.5_f32));
    }

    #[test]
    fn no_temperature_node_returns_none() {
        let json = r#"{
            "Text": "Computer",
            "Value": "",
            "Children": [
                {
                    "Text": "MYPC",
                    "Value": "",
                    "Children": [
                        { "Text": "Fan #1", "Value": "1200 RPM", "Children": [] },
                        { "Text": "Voltage", "Value": "1.2 V", "Children": [] }
                    ]
                }
            ]
        }"#;
        let root: LhmNode = serde_json::from_str(json).unwrap();
        assert_eq!(extract_cpu_temp(&root), None);
    }

    #[test]
    fn falls_back_to_cpu_package_when_tctl_missing() {
        let json = r#"{
            "Text": "Computer",
            "Value": "",
            "Children": [
                { "Text": "CPU Package", "Value": "58.0 °C", "Children": [] }
            ]
        }"#;
        let root: LhmNode = serde_json::from_str(json).unwrap();
        assert_eq!(extract_cpu_temp(&root), Some(58.0_f32));
    }
}
