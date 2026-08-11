//! config.toml：daemon 是唯一 owner（CLAUDE.md §10，shell 不读文件）。路径
//! `%APPDATA%\auxdeck\config.toml`；不存在则写出带注释的默认文件。运行期用
//! `notify` 监听所在目录，防抖 ~500ms 后重新解析；解析失败保留旧配置并
//! `warn`，绝不崩（CLAUDE.md 任务书 1）。
//!
//! 生效面：采样间隔（system/app_usage/weather）随 `AppConfig` 的
//! `watch::Receiver` 热生效；grid/widgets 变更时直接经 `Hub` 重推
//! `Push::Config` 给所有已连接客户端。

use std::path::{Path, PathBuf};
use std::time::Duration;

use auxdeck_core::{GridConfig, Push, ShellConfig, WidgetPlacement};
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::hub::Hub;

/// 运行期配置：由 `config.toml` 解析并转换而来，供各 collector / rpc 使用。
#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub sampling: SamplingConfig,
    pub weather: WeatherConfig,
    pub shell: ShellConfig,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingConfig {
    pub system: Duration,
    pub app_usage: Duration,
    pub weather: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherConfig {
    pub enabled: bool,
    pub provider: WeatherProviderKind,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub city_label: String,
    pub qweather_key: Option<String>,
}

impl WeatherConfig {
    /// 经纬度均已配置时返回 `Some`；否则天气 collector 视为不可用
    /// （CLAUDE.md §7.1：不做 IP 定位，必须手动配置）。
    pub fn coords(&self) -> Option<(f64, f64)> {
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherProviderKind {
    OpenMeteo,
    QWeather,
}

fn auxdeck_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("auxdeck")
}

pub fn config_path() -> PathBuf {
    auxdeck_dir().join("config.toml")
}

// ---- TOML 原始结构：每个字段都有默认值，允许用户只写部分字段 ----

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
struct RawSampling {
    system_ms: u64,
    app_usage_ms: u64,
    weather_min: u64,
}

impl Default for RawSampling {
    fn default() -> Self {
        Self {
            system_ms: 1000,
            app_usage_ms: 5000,
            weather_min: 10,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
struct RawWeather {
    enabled: bool,
    provider: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
    city_label: String,
    qweather_key: Option<String>,
}

impl Default for RawWeather {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "open-meteo".to_string(),
            latitude: None,
            longitude: None,
            city_label: String::new(),
            qweather_key: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
struct RawGrid {
    columns: u32,
    row_height_px: u32,
    gap_px: u32,
}

impl Default for RawGrid {
    fn default() -> Self {
        let g = GridConfig::default();
        Self {
            columns: g.columns,
            row_height_px: g.row_height_px,
            gap_px: g.gap_px,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RawWidget {
    kind: String,
    page: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl From<WidgetPlacement> for RawWidget {
    fn from(w: WidgetPlacement) -> Self {
        Self {
            kind: w.kind,
            page: w.page,
            x: w.x,
            y: w.y,
            w: w.w,
            h: w.h,
        }
    }
}

impl From<RawWidget> for WidgetPlacement {
    fn from(w: RawWidget) -> Self {
        Self {
            kind: w.kind,
            page: w.page,
            x: w.x,
            y: w.y,
            w: w.w,
            h: w.h,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
struct RawConfig {
    sampling: RawSampling,
    weather: RawWeather,
    grid: RawGrid,
    widgets: Vec<RawWidget>,
}

impl Default for RawConfig {
    fn default() -> Self {
        Self {
            sampling: RawSampling::default(),
            weather: RawWeather::default(),
            grid: RawGrid::default(),
            widgets: ShellConfig::default()
                .widgets
                .into_iter()
                .map(RawWidget::from)
                .collect(),
        }
    }
}

impl RawConfig {
    fn into_app_config(self) -> AppConfig {
        let provider = match self.weather.provider.as_str() {
            "qweather" => WeatherProviderKind::QWeather,
            _ => WeatherProviderKind::OpenMeteo,
        };
        let widgets = if self.widgets.is_empty() {
            ShellConfig::default().widgets
        } else {
            self.widgets
                .into_iter()
                .map(WidgetPlacement::from)
                .collect()
        };

        AppConfig {
            sampling: SamplingConfig {
                // 下限保护：配置里填 0 或极小值时不至于让采样循环空转打满 CPU。
                system: Duration::from_millis(self.sampling.system_ms.max(100)),
                app_usage: Duration::from_millis(self.sampling.app_usage_ms.max(500)),
                weather: Duration::from_secs(self.sampling.weather_min.max(1) * 60),
            },
            weather: WeatherConfig {
                enabled: self.weather.enabled,
                provider,
                latitude: self.weather.latitude,
                longitude: self.weather.longitude,
                city_label: self.weather.city_label,
                qweather_key: self.weather.qweather_key,
            },
            shell: ShellConfig {
                grid: GridConfig {
                    columns: self.grid.columns,
                    row_height_px: self.grid.row_height_px,
                    gap_px: self.grid.gap_px,
                },
                widgets,
            },
        }
    }
}

/// 生成带注释的默认 `config.toml` 文本。数值全部取自各 `Default` 实现（含
/// `auxdeck_core::ShellConfig::default()` 的四张默认 widget），避免注释文本与
/// 实际默认值日后漂移不一致。
fn default_toml_text() -> String {
    let sampling = RawSampling::default();
    let weather = RawWeather::default();
    let grid = RawGrid::default();

    let mut out = format!(
        "# auxdeck daemon 配置文件。daemon 启动时监听本文件变更并热重载，\n\
         # 大部分改动几秒内生效，无需重启进程。\n\
         # 语法错误时 daemon 会保留上一份可用配置并在日志中记录警告。\n\
         \n\
         [sampling]\n\
         # 系统指标（CPU/GPU 利用率与温度）采样间隔，毫秒\n\
         system_ms = {system_ms}\n\
         # 前台窗口轮询间隔，毫秒（用于统计应用使用时长）\n\
         app_usage_ms = {app_usage_ms}\n\
         # 天气刷新间隔，分钟\n\
         weather_min = {weather_min}\n\
         \n\
         [weather]\n\
         # 是否启用天气 widget；未同时配置 latitude/longitude 时即使为 true 也不会请求（不做 IP 定位）\n\
         enabled = {enabled}\n\
         # \"open-meteo\"（免 key，默认）或 \"qweather\"（需 qweather_key）\n\
         provider = \"{provider}\"\n\
         # 纬度 / 经度，例如上海约为 31.23 / 121.47；取消注释并按实际位置填写\n\
         # latitude = 31.23\n\
         # longitude = 121.47\n\
         # 面板上显示的城市名\n\
         city_label = \"{city_label}\"\n\
         # 和风天气 key，仅 provider = \"qweather\" 时需要，https://dev.qweather.com/ 免费申请\n\
         # qweather_key = \"\"\n\
         \n\
         [grid]\n\
         columns = {columns}\n\
         row_height_px = {row_height_px}\n\
         gap_px = {gap_px}\n",
        system_ms = sampling.system_ms,
        app_usage_ms = sampling.app_usage_ms,
        weather_min = sampling.weather_min,
        enabled = weather.enabled,
        provider = weather.provider,
        city_label = weather.city_label,
        columns = grid.columns,
        row_height_px = grid.row_height_px,
        gap_px = grid.gap_px,
    );

    for widget in ShellConfig::default().widgets {
        out.push_str(&format!(
            "\n[[widgets]]\nkind = \"{}\"\npage = {}\nx = {}\ny = {}\nw = {}\nh = {}\n",
            widget.kind, widget.page, widget.x, widget.y, widget.w, widget.h
        ));
    }

    out
}

fn write_default_config(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            warn!("failed to create config dir {}: {err}", parent.display());
            return;
        }
    }
    if let Err(err) = std::fs::write(path, default_toml_text()) {
        warn!("failed to write default config {}: {err}", path.display());
    }
}

/// 加载 `config.toml`；不存在则写出默认文件并返回默认配置；解析失败则记录
/// error 并回退到内置默认值（不覆盖磁盘上已有的——可能只是临时手误——文件）。
pub fn load_or_init() -> AppConfig {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<RawConfig>(&text) {
            Ok(raw) => {
                info!("loaded config from {}", path.display());
                raw.into_app_config()
            }
            Err(err) => {
                error!(
                    "failed to parse {}: {err}; falling back to built-in defaults",
                    path.display()
                );
                RawConfig::default().into_app_config()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            info!("no config at {}, writing defaults", path.display());
            write_default_config(&path);
            RawConfig::default().into_app_config()
        }
        Err(err) => {
            error!(
                "failed to read {}: {err}; falling back to built-in defaults",
                path.display()
            );
            RawConfig::default().into_app_config()
        }
    }
}

fn reload(path: &Path, tx: &watch::Sender<AppConfig>, hub: &Hub) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            warn!(
                "config reload: failed to read {}: {err}; keeping previous config",
                path.display()
            );
            return;
        }
    };
    let raw = match toml::from_str::<RawConfig>(&text) {
        Ok(raw) => raw,
        Err(err) => {
            warn!(
                "config reload: failed to parse {}: {err}; keeping previous config",
                path.display()
            );
            return;
        }
    };

    let cfg = raw.into_app_config();
    info!("config reloaded from {}", path.display());
    hub.publish(Push::Config(cfg.shell.clone()));
    let _ = tx.send(cfg);
}

/// 是否是本次要关心的文件事件：路径命中 + 事件确实修改了内容（含编辑器
/// "删除后重建" 的保存方式，Create/Modify/Remove 都算数，交由 debounce 合并）。
fn event_touches(event: &notify::Result<notify::Event>, target: &Path) -> bool {
    match event {
        Ok(event) => event.paths.iter().any(|p| p == target),
        Err(err) => {
            warn!("config watcher error: {err}");
            false
        }
    }
}

/// 启动 `config.toml` 的文件监听线程，返回随文件变更热更新的
/// `watch::Receiver<AppConfig>`。监听所在目录而非文件本身：常见编辑器保存
/// 方式是"删除+重建"，直接监听单个文件在这种情况下会失效。
///
/// notify 初始化失败（如目录不可访问）时记录 warn 并放弃热重载——不影响已
/// 加载的 `initial` 配置继续工作，只是后续文件变更不会被感知。
pub fn spawn_watcher(initial: AppConfig, hub: Hub) -> watch::Receiver<AppConfig> {
    let (tx, rx) = watch::channel(initial);
    let path = config_path();

    std::thread::spawn(move || watch_thread(path, tx, hub));

    rx
}

fn watch_thread(path: PathBuf, tx: watch::Sender<AppConfig>, hub: Hub) {
    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(notify_tx) {
        Ok(w) => w,
        Err(err) => {
            warn!("failed to create config watcher: {err}; hot reload disabled");
            return;
        }
    };

    let watch_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    if let Err(err) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
        warn!(
            "failed to watch {}: {err}; hot reload disabled",
            watch_dir.display()
        );
        return;
    }
    info!("watching {} for config changes", path.display());

    loop {
        let first = match notify_rx.recv() {
            Ok(event) => event,
            Err(_) => return, // watcher (含内部线程) 已被丢弃
        };
        let mut relevant = event_touches(&first, &path);

        // 防抖：500ms 静默窗口内持续吸收后续事件，只在窗口结束后重载一次。
        loop {
            match notify_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(event) => relevant |= event_touches(&event, &path),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }

        if relevant {
            reload(&path, &tx, &hub);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let text = r#"
            [sampling]
            system_ms = 2000
            app_usage_ms = 6000
            weather_min = 15

            [weather]
            enabled = true
            provider = "qweather"
            latitude = 31.23
            longitude = 121.47
            city_label = "上海"
            qweather_key = "abc123"

            [grid]
            columns = 4
            row_height_px = 150
            gap_px = 10

            [[widgets]]
            kind = "clock"
            page = 0
            x = 0
            y = 0
            w = 4
            h = 3
        "#;
        let raw: RawConfig = toml::from_str(text).unwrap();
        let cfg = raw.into_app_config();

        assert_eq!(cfg.sampling.system, Duration::from_millis(2000));
        assert_eq!(cfg.sampling.app_usage, Duration::from_millis(6000));
        assert_eq!(cfg.sampling.weather, Duration::from_secs(15 * 60));
        assert!(cfg.weather.enabled);
        assert_eq!(cfg.weather.provider, WeatherProviderKind::QWeather);
        assert_eq!(cfg.weather.coords(), Some((31.23, 121.47)));
        assert_eq!(cfg.weather.city_label, "上海");
        assert_eq!(cfg.shell.widgets.len(), 1);
        assert_eq!(cfg.shell.widgets[0].kind, "clock");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let raw: RawConfig = toml::from_str("").unwrap();
        let cfg = raw.into_app_config();

        assert_eq!(cfg.sampling.system, Duration::from_millis(1000));
        assert!(!cfg.weather.enabled);
        assert_eq!(cfg.weather.provider, WeatherProviderKind::OpenMeteo);
        assert_eq!(cfg.weather.coords(), None);
        // 空 widgets 数组回退到 auxdeck_core 的默认四张卡，而不是空白面板。
        assert_eq!(cfg.shell.widgets, ShellConfig::default().widgets);
    }

    #[test]
    fn invalid_toml_syntax_is_rejected() {
        let result = toml::from_str::<RawConfig>("this is not valid toml {{{");
        assert!(result.is_err());
    }

    #[test]
    fn unknown_provider_string_falls_back_to_open_meteo() {
        let text = r#"
            [weather]
            provider = "something-else"
        "#;
        let raw: RawConfig = toml::from_str(text).unwrap();
        let cfg = raw.into_app_config();
        assert_eq!(cfg.weather.provider, WeatherProviderKind::OpenMeteo);
    }

    #[test]
    fn zero_intervals_are_clamped_to_a_safe_minimum() {
        let text = r#"
            [sampling]
            system_ms = 0
            app_usage_ms = 0
            weather_min = 0
        "#;
        let raw: RawConfig = toml::from_str(text).unwrap();
        let cfg = raw.into_app_config();
        assert!(cfg.sampling.system >= Duration::from_millis(100));
        assert!(cfg.sampling.app_usage >= Duration::from_millis(500));
        assert!(cfg.sampling.weather >= Duration::from_secs(60));
    }

    #[test]
    fn default_toml_text_round_trips_through_parser() {
        let text = default_toml_text();
        let raw: RawConfig = toml::from_str(&text).expect("default config must parse");
        let cfg = raw.into_app_config();
        assert_eq!(cfg.shell.widgets, ShellConfig::default().widgets);
        assert_eq!(cfg.shell.grid, GridConfig::default());
    }
}
