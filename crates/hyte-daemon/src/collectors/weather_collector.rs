//! 天气采集（CLAUDE.md §7.1）：按 `config.weather.provider` 选择 provider，
//! 轮询间隔 `weather_min`（默认 10min），启动即采一次。`weather.enabled =
//! false` 或未配置经纬度时 collector 完全不发请求、不推 `Push::Weather`
//! （不做 IP 定位）。请求失败保留上一次快照不清空，只在可用性状态变化时
//! 打日志，避免刷屏（与 lhm/nvml collector 同样的失败隔离原则）。

use std::time::Duration;

use async_trait::async_trait;
use hyte_core::{Push, WeatherSnapshot};
use serde::Deserialize;
use tokio::sync::watch;
use tracing::{info, warn};

use super::AvailabilityState;
use crate::config::{AppConfig, WeatherConfig, WeatherProviderKind};
use crate::hub::Hub;

/// 每次天气请求的超时（共享 http_client 无默认超时，须 per-request 指定）。
const WEATHER_TIMEOUT: Duration = Duration::from_secs(10);

const OPEN_METEO_URL: &str = "https://api.open-meteo.com/v1/forecast";
const QWEATHER_NOW_URL: &str = "https://devapi.qweather.com/v7/weather/now";
const QWEATHER_3D_URL: &str = "https://devapi.qweather.com/v7/weather/3d";

#[derive(Debug)]
enum WeatherError {
    Http(reqwest::Error),
    Api(String),
}

impl std::fmt::Display for WeatherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WeatherError::Http(err) => write!(f, "http error: {err}"),
            WeatherError::Api(msg) => write!(f, "api error: {msg}"),
        }
    }
}

impl From<reqwest::Error> for WeatherError {
    fn from(err: reqwest::Error) -> Self {
        WeatherError::Http(err)
    }
}

#[async_trait]
trait WeatherProvider: Send + Sync {
    async fn fetch(&self, client: &reqwest::Client) -> Result<WeatherSnapshot, WeatherError>;
}

fn build_provider(cfg: &WeatherConfig, lat: f64, lon: f64) -> Option<Box<dyn WeatherProvider>> {
    match cfg.provider {
        WeatherProviderKind::OpenMeteo => Some(Box::new(OpenMeteoProvider {
            latitude: lat,
            longitude: lon,
            city_label: cfg.city_label.clone(),
        })),
        WeatherProviderKind::QWeather => {
            let key = cfg.qweather_key.clone().filter(|k| !k.is_empty())?;
            Some(Box::new(QWeatherProvider {
                latitude: lat,
                longitude: lon,
                city_label: cfg.city_label.clone(),
                key,
            }))
        }
    }
}

/// 采集主循环：每次迭代先按当前配置判断是否应该请求，请求间隔与
/// enabled/provider/坐标 全部实时读取 `cfg_rx`，因此天气开关、坐标、
/// provider 切换都无需重启进程。
pub async fn run(mut cfg_rx: watch::Receiver<AppConfig>, hub: Hub) {
    // 共享全进程 HTTP 客户端（连接池/TLS 只留一份），超时在各 fetch 内 per-request 指定。
    let client = super::http_client().clone();
    let mut availability = AvailabilityState::new();
    let mut idle_reason: Option<&'static str> = None;

    loop {
        let (weather_cfg, interval) = {
            let cfg = cfg_rx.borrow();
            (cfg.weather.clone(), cfg.sampling.weather)
        };

        let Some((lat, lon)) = weather_cfg.coords() else {
            log_idle_once(&mut idle_reason, "latitude/longitude not configured");
            if cfg_rx.changed().await.is_err() {
                break;
            }
            continue;
        };
        if !weather_cfg.enabled {
            log_idle_once(&mut idle_reason, "weather.enabled = false");
            if cfg_rx.changed().await.is_err() {
                break;
            }
            continue;
        }
        let Some(provider) = build_provider(&weather_cfg, lat, lon) else {
            log_idle_once(
                &mut idle_reason,
                "qweather selected but qweather_key missing",
            );
            if cfg_rx.changed().await.is_err() {
                break;
            }
            continue;
        };
        idle_reason = None;

        match provider.fetch(&client).await {
            Ok(snapshot) => {
                if availability.note(true) == Some(true) {
                    info!("weather provider reachable");
                }
                hub.publish(Push::Weather(snapshot));
            }
            Err(err) => {
                if availability.note(false).is_some() {
                    warn!("weather fetch failed, keeping last snapshot: {err}");
                }
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            changed = cfg_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
}

fn log_idle_once(state: &mut Option<&'static str>, reason: &'static str) {
    if *state != Some(reason) {
        info!("weather collector idle: {reason}");
        *state = Some(reason);
    }
}

// ---- Open-Meteo（免 key，默认）----

struct OpenMeteoProvider {
    latitude: f64,
    longitude: f64,
    city_label: String,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    current: OpenMeteoCurrent,
    daily: OpenMeteoDaily,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoCurrent {
    temperature_2m: f32,
    weather_code: u32,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoDaily {
    #[serde(default)]
    temperature_2m_max: Vec<f32>,
    #[serde(default)]
    temperature_2m_min: Vec<f32>,
    #[serde(default)]
    sunrise: Vec<String>,
    #[serde(default)]
    sunset: Vec<String>,
}

#[async_trait]
impl WeatherProvider for OpenMeteoProvider {
    async fn fetch(&self, client: &reqwest::Client) -> Result<WeatherSnapshot, WeatherError> {
        let resp: OpenMeteoResponse = client
            .get(OPEN_METEO_URL)
            .query(&[
                ("latitude", self.latitude.to_string()),
                ("longitude", self.longitude.to_string()),
                ("current", "temperature_2m,weather_code".to_string()),
                (
                    "daily",
                    "temperature_2m_max,temperature_2m_min,sunrise,sunset".to_string(),
                ),
                ("timezone", "auto".to_string()),
            ])
            .timeout(WEATHER_TIMEOUT)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let temp_c = resp.current.temperature_2m;
        Ok(WeatherSnapshot {
            ts_ms: crate::now_ms(),
            city: self.city_label.clone(),
            temp_c,
            condition: wmo_to_condition(resp.current.weather_code).to_string(),
            high_c: resp
                .daily
                .temperature_2m_max
                .first()
                .copied()
                .unwrap_or(temp_c),
            low_c: resp
                .daily
                .temperature_2m_min
                .first()
                .copied()
                .unwrap_or(temp_c),
            sunrise: extract_hhmm(resp.daily.sunrise.first()),
            sunset: extract_hhmm(resp.daily.sunset.first()),
        })
    }
}

/// Open-Meteo 的 sunrise/sunset 是不带时区偏移的本地 ISO 时间
/// （`timezone=auto` 已经换算），形如 `"2026-08-11T05:43"`；取 `T` 后的
/// `"HH:MM"` 即可，无需再解析。
fn extract_hhmm(iso: Option<&String>) -> String {
    iso.and_then(|s| s.split('T').nth(1))
        .map(|t| t.chars().take(5).collect())
        .unwrap_or_else(|| "--:--".to_string())
}

/// WMO weather_code → 中文天况文案。覆盖 Open-Meteo 文档列出的常见码；未知
/// 码统一归为"未知"，不影响其余字段展示。
fn wmo_to_condition(code: u32) -> &'static str {
    match code {
        0 | 1 => "晴",
        2 => "多云",
        3 => "阴",
        45 | 48 => "雾",
        51 | 53 => "小雨",
        55 => "中雨",
        56 | 57 => "冻雨",
        61 => "小雨",
        63 => "中雨",
        65 => "大雨",
        66 | 67 => "冻雨",
        71 => "小雪",
        73 => "中雪",
        75 => "大雪",
        77 => "雪粒",
        80 => "阵雨",
        81 => "阵雨",
        82 => "强阵雨",
        85 => "阵雪",
        86 => "大阵雪",
        95 => "雷阵雨",
        96 | 99 => "雷阵雨伴冰雹",
        _ => "未知",
    }
}

// ---- QWeather（和风天气，需 key）----

struct QWeatherProvider {
    latitude: f64,
    longitude: f64,
    city_label: String,
    key: String,
}

#[derive(Debug, Deserialize)]
struct QWeatherNowResponse {
    code: String,
    now: Option<QWeatherNow>,
}

#[derive(Debug, Deserialize)]
struct QWeatherNow {
    temp: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct QWeatherDailyResponse {
    code: String,
    daily: Option<Vec<QWeatherDaily>>,
}

#[derive(Debug, Deserialize)]
struct QWeatherDaily {
    #[serde(rename = "tempMax")]
    temp_max: String,
    #[serde(rename = "tempMin")]
    temp_min: String,
    sunrise: String,
    sunset: String,
}

#[async_trait]
impl WeatherProvider for QWeatherProvider {
    async fn fetch(&self, client: &reqwest::Client) -> Result<WeatherSnapshot, WeatherError> {
        // 和风天气坐标顺序是 "经度,纬度"，与常见的 "纬度,经度" 相反。
        let location = format!("{},{}", self.longitude, self.latitude);

        let now_resp: QWeatherNowResponse = client
            .get(QWEATHER_NOW_URL)
            .query(&[("location", location.as_str()), ("key", self.key.as_str())])
            .timeout(WEATHER_TIMEOUT)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if now_resp.code != "200" {
            return Err(WeatherError::Api(format!(
                "qweather now: code {}",
                now_resp.code
            )));
        }
        let now = now_resp
            .now
            .ok_or_else(|| WeatherError::Api("qweather now: missing `now` field".to_string()))?;

        let daily_resp: QWeatherDailyResponse = client
            .get(QWEATHER_3D_URL)
            .query(&[("location", location.as_str()), ("key", self.key.as_str())])
            .timeout(WEATHER_TIMEOUT)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if daily_resp.code != "200" {
            return Err(WeatherError::Api(format!(
                "qweather 3d: code {}",
                daily_resp.code
            )));
        }
        let today = daily_resp
            .daily
            .and_then(|days| days.into_iter().next())
            .ok_or_else(|| WeatherError::Api("qweather 3d: empty daily list".to_string()))?;

        let temp_c = now
            .temp
            .parse::<f32>()
            .map_err(|_| WeatherError::Api(format!("qweather now: bad temp {:?}", now.temp)))?;
        let high_c = today.temp_max.parse::<f32>().unwrap_or(temp_c);
        let low_c = today.temp_min.parse::<f32>().unwrap_or(temp_c);

        Ok(WeatherSnapshot {
            ts_ms: crate::now_ms(),
            city: self.city_label.clone(),
            temp_c,
            condition: now.text,
            high_c,
            low_c,
            sunrise: today.sunrise,
            sunset: today.sunset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wmo_covers_each_condition_family() {
        assert_eq!(wmo_to_condition(0), "晴");
        assert_eq!(wmo_to_condition(1), "晴");
        assert_eq!(wmo_to_condition(2), "多云");
        assert_eq!(wmo_to_condition(3), "阴");
        assert_eq!(wmo_to_condition(45), "雾");
        assert_eq!(wmo_to_condition(48), "雾");
        assert_eq!(wmo_to_condition(51), "小雨");
        assert_eq!(wmo_to_condition(61), "小雨");
        assert_eq!(wmo_to_condition(63), "中雨");
        assert_eq!(wmo_to_condition(65), "大雨");
        assert_eq!(wmo_to_condition(56), "冻雨");
        assert_eq!(wmo_to_condition(66), "冻雨");
        assert_eq!(wmo_to_condition(71), "小雪");
        assert_eq!(wmo_to_condition(73), "中雪");
        assert_eq!(wmo_to_condition(75), "大雪");
        assert_eq!(wmo_to_condition(77), "雪粒");
        assert_eq!(wmo_to_condition(80), "阵雨");
        assert_eq!(wmo_to_condition(82), "强阵雨");
        assert_eq!(wmo_to_condition(85), "阵雪");
        assert_eq!(wmo_to_condition(95), "雷阵雨");
        assert_eq!(wmo_to_condition(99), "雷阵雨伴冰雹");
    }

    #[test]
    fn wmo_unknown_code_falls_back() {
        assert_eq!(wmo_to_condition(12345), "未知");
    }

    #[test]
    fn extract_hhmm_from_iso_datetime() {
        let s = "2026-08-11T05:43".to_string();
        assert_eq!(extract_hhmm(Some(&s)), "05:43");
    }

    #[test]
    fn extract_hhmm_missing_returns_placeholder() {
        assert_eq!(extract_hhmm(None), "--:--");
    }

    #[test]
    fn open_meteo_response_deserializes() {
        let json = r#"{
            "current": {"temperature_2m": 25.3, "weather_code": 2},
            "daily": {
                "temperature_2m_max": [32.1],
                "temperature_2m_min": [20.5],
                "sunrise": ["2026-08-11T05:43"],
                "sunset": ["2026-08-11T18:52"]
            }
        }"#;
        let resp: OpenMeteoResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.current.weather_code, 2);
        assert_eq!(resp.daily.sunrise[0], "2026-08-11T05:43");
    }

    #[test]
    fn qweather_now_response_deserializes() {
        let json = r#"{"code":"200","now":{"temp":"25","text":"晴"}}"#;
        let resp: QWeatherNowResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, "200");
        assert_eq!(resp.now.unwrap().text, "晴");
    }

    #[test]
    fn qweather_daily_response_deserializes() {
        let json = r#"{"code":"200","daily":[{"tempMax":"32","tempMin":"20","sunrise":"05:43","sunset":"18:52"}]}"#;
        let resp: QWeatherDailyResponse = serde_json::from_str(json).unwrap();
        let today = &resp.daily.unwrap()[0];
        assert_eq!(today.temp_max, "32");
        assert_eq!(today.sunrise, "05:43");
    }

    #[test]
    fn build_provider_qweather_without_key_is_none() {
        let cfg = WeatherConfig {
            enabled: true,
            provider: WeatherProviderKind::QWeather,
            latitude: Some(31.23),
            longitude: Some(121.47),
            city_label: "上海".to_string(),
            qweather_key: None,
        };
        assert!(build_provider(&cfg, 31.23, 121.47).is_none());
    }

    #[test]
    fn build_provider_open_meteo_always_available() {
        let cfg = WeatherConfig {
            enabled: true,
            provider: WeatherProviderKind::OpenMeteo,
            latitude: Some(31.23),
            longitude: Some(121.47),
            city_label: "上海".to_string(),
            qweather_key: None,
        };
        assert!(build_provider(&cfg, 31.23, 121.47).is_some());
    }
}
