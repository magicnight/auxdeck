//! HyteDeck 跨进程共享类型。daemon ↔ shell ↔ hostage 的所有数据结构定义在这里（CLAUDE.md §10）。
//! daemon 是 config.toml 的唯一 owner；shell 不读文件，布局与数据全部经 WebSocket 推送获得。

use serde::{Deserialize, Serialize};

/// RPC 默认监听地址，仅本机回环（CLAUDE.md §4）。
pub const RPC_ADDR: &str = "127.0.0.1:9600";

/// daemon 推送的系统指标快照（M1 最小集）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    /// Unix 毫秒时间戳，shell 据此判断数据新鲜度。
    pub ts_ms: u64,
    /// CPU 整体利用率，0.0–100.0。
    pub cpu_usage: f32,
    /// CPU 温度（℃）。LHM 不可用时为 None（CLAUDE.md §4）。
    pub cpu_temp: Option<f32>,
    /// GPU 利用率，0.0–100.0。NVML 不可用时为 None。
    pub gpu_usage: Option<f32>,
    /// GPU 温度（℃）。NVML 不可用时为 None。
    pub gpu_temp: Option<f32>,
}

/// 网格布局参数（CLAUDE.md §8.1：4 列 × n 行 + 多页，682px ÷ 4 ≈ 170px/列）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridConfig {
    pub columns: u32,
    pub row_height_px: u32,
    pub gap_px: u32,
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            columns: 4,
            row_height_px: 160,
            gap_px: 12,
        }
    }
}

/// 单个 widget 在网格上的占位（CLAUDE.md §8.1 的 `(page, x, y, w, h)`）。
/// `kind` 用字符串而非枚举：shell 忽略未知 kind，新增 widget 不破坏旧 shell。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetPlacement {
    pub kind: String,
    pub page: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// 推给 shell 的渲染配置。config.toml 变更热重载后 daemon 重新推送。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellConfig {
    pub grid: GridConfig,
    pub widgets: Vec<WidgetPlacement>,
}

impl Default for ShellConfig {
    /// 默认布局对齐 Nexus 第 1 页观感（CLAUDE.md §12）。shell 断连 / 未收到
    /// Config 推送前也可用该默认值渲染。
    fn default() -> Self {
        let widget = |kind: &str, page: u32, x: u32, y: u32, w: u32, h: u32| WidgetPlacement {
            kind: kind.to_string(),
            page,
            x,
            y,
            w,
            h,
        };
        Self {
            grid: GridConfig::default(),
            widgets: vec![
                widget("clock", 0, 0, 0, 4, 3),
                widget("metrics", 0, 0, 3, 4, 4),
                widget("weather", 0, 0, 7, 4, 4),
                widget("app_usage", 0, 0, 11, 4, 4),
            ],
        }
    }
}

/// 天气快照（CLAUDE.md §7.1）。日出日落已按本地系统时区换算为 "HH:MM"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherSnapshot {
    pub ts_ms: u64,
    /// config 中用户配置的显示名（城市名），不做 IP 定位。
    pub city: String,
    pub temp_c: f32,
    /// 中文天况文案（晴 / 多云 / 小雨…）。
    pub condition: String,
    pub high_c: f32,
    pub low_c: f32,
    pub sunrise: String,
    pub sunset: String,
}

/// 单个应用的前台使用时长（CLAUDE.md §7.4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUsageEntry {
    /// 进程名（不含扩展名），如 "firefox"。
    pub name: String,
    pub today_secs: u64,
    pub yesterday_secs: u64,
}

/// 应用使用时长快照：今日 Top N + 昨日对比。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUsageSnapshot {
    pub ts_ms: u64,
    pub top: Vec<AppUsageEntry>,
}

/// daemon → 客户端的 WebSocket 推送信封。
/// JSON 形如：`{"type":"system","data":{…}}`。
/// 客户端连接建立时，daemon 立即推送 Config 与各类最新快照（有则推）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Push {
    System(SystemSnapshot),
    Weather(WeatherSnapshot),
    AppUsage(AppUsageSnapshot),
    Config(ShellConfig),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(push: &Push) -> Push {
        serde_json::from_str(&serde_json::to_string(push).unwrap()).unwrap()
    }

    #[test]
    fn push_system_json_shape() {
        let push = Push::System(SystemSnapshot {
            ts_ms: 1,
            cpu_usage: 12.5,
            cpu_temp: None,
            gpu_usage: Some(4.0),
            gpu_temp: Some(38.0),
        });
        let json = serde_json::to_string(&push).unwrap();
        assert!(json.contains(r#""type":"system""#));
        assert!(json.contains(r#""cpu_temp":null"#));
        assert_eq!(roundtrip(&push), push);
    }

    #[test]
    fn push_config_json_shape() {
        let push = Push::Config(ShellConfig::default());
        let json = serde_json::to_string(&push).unwrap();
        assert!(json.contains(r#""type":"config""#));
        assert!(json.contains(r#""kind":"clock""#));
        assert_eq!(roundtrip(&push), push);
    }

    #[test]
    fn push_weather_and_app_usage_roundtrip() {
        let weather = Push::Weather(WeatherSnapshot {
            ts_ms: 2,
            city: "上海".into(),
            temp_c: 25.0,
            condition: "晴".into(),
            high_c: 32.0,
            low_c: 20.2,
            sunrise: "05:45".into(),
            sunset: "18:11".into(),
        });
        assert_eq!(roundtrip(&weather), weather);

        let usage = Push::AppUsage(AppUsageSnapshot {
            ts_ms: 3,
            top: vec![AppUsageEntry {
                name: "firefox".into(),
                today_secs: 2557,
                yesterday_secs: 1200,
            }],
        });
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains(r#""type":"app_usage""#));
        assert_eq!(roundtrip(&usage), usage);
    }
}
