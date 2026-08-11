//! 应用使用时长采集（CLAUDE.md §7.4）：每 `app_usage_ms` 取一次前台窗口
//! 所属进程名，累计到当日总时长；周期性落盘（有变更才写，最多每 60s 一次）
//! 并推送 `Push::AppUsage`；跨日自动滚动。纯聚合/滚动逻辑在
//! `app_usage_tracker`（可脱离 Windows API 单测），本文件只负责系统调用、
//! 文件 IO 与调度。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use hyte_core::Push;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::warn;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use super::app_usage_tracker::UsageTracker;
use crate::config::AppConfig;
use crate::hub::Hub;

const PERSIST_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedDay {
    #[serde(default)]
    totals: HashMap<String, u64>,
}

fn usage_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("HyteDeck")
        .join("usage")
}

fn day_file(day: NaiveDate) -> PathBuf {
    usage_dir().join(format!("{}.json", day.format("%Y-%m-%d")))
}

/// 载入指定日期的持久化数据；文件不存在是正常情况（新的一天 / 首次运行），
/// 静默返回空表。解析失败（文件损坏）记录 warn 后同样以空表继续——不能因为
/// 一份坏文件让 collector 罢工。
fn load_day(day: NaiveDate) -> HashMap<String, u64> {
    let path = day_file(day);
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<PersistedDay>(&text) {
            Ok(parsed) => parsed.totals,
            Err(err) => {
                warn!(
                    "failed to parse usage file {}: {err}; starting empty",
                    path.display()
                );
                HashMap::new()
            }
        },
        Err(_) => HashMap::new(),
    }
}

fn save_day(day: NaiveDate, totals: &HashMap<String, u64>) {
    let dir = usage_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        warn!("failed to create usage dir {}: {err}", dir.display());
        return;
    }
    let path = day_file(day);
    let payload = PersistedDay {
        totals: totals.clone(),
    };
    match serde_json::to_string_pretty(&payload) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                warn!("failed to write usage file {}: {err}", path.display());
            }
        }
        Err(err) => warn!("failed to serialize usage totals: {err}"),
    }
}

/// 取前台窗口所属进程名（不含 `.exe` 扩展名）。任何一步失败都返回
/// `None`——采集失败不得让整个 loop 崩溃，与其余 collector 的失败隔离原则
/// 一致。
fn foreground_process_name(sys: &mut System) -> Option<String> {
    let hwnd: HWND = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
    if pid == 0 {
        return None;
    }

    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing(),
    );
    sys.process(pid).map(|p| {
        let name = p.name().to_string_lossy().into_owned();
        name.strip_suffix(".exe")
            .map(str::to_string)
            .unwrap_or(name)
    })
}

pub async fn run(mut cfg_rx: watch::Receiver<AppConfig>, hub: Hub) {
    let today = Local::now().date_naive();
    let today_totals = load_day(today);
    let yesterday_totals = load_day(today.pred_opt().unwrap_or(today));
    let mut tracker = UsageTracker::new(today, today_totals, yesterday_totals);

    let mut sys = System::new();
    let mut dirty = false;
    let mut last_persist = Instant::now();

    // 启动即推一次当前状态（哪怕全 0），shell 不必等首个持久化窗口才有数据。
    hub.publish(Push::AppUsage(tracker.snapshot(crate::now_ms())));

    loop {
        let interval = cfg_rx.borrow().sampling.app_usage;
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                let now = Local::now();
                let process = foreground_process_name(&mut sys);
                if process.is_some() {
                    dirty = true;
                }
                tracker.record(now.date_naive(), process.as_deref(), interval);

                if dirty && last_persist.elapsed() >= PERSIST_INTERVAL {
                    save_day(tracker.day(), tracker.today());
                    hub.publish(Push::AppUsage(tracker.snapshot(crate::now_ms())));
                    dirty = false;
                    last_persist = Instant::now();
                }
            }
            changed = cfg_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
}
