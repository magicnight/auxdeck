//! 应用使用时长的纯聚合逻辑（CLAUDE.md §7.4）：只做内存计数与跨日滚动，
//! 不涉及任何系统调用或文件 IO —— "当前日期"完全由调用方传入，因此可以
//! 完全脱离 Windows API 与文件系统单测跨日滚动路径（任务书对时间可注入的
//! 单测要求）。

use std::collections::HashMap;
use std::time::Duration;

use auxdeck_core::{AppUsageEntry, AppUsageSnapshot};
use chrono::NaiveDate;

const TOP_N: usize = 5;

pub struct UsageTracker {
    day: NaiveDate,
    today: HashMap<String, u64>,
    yesterday: HashMap<String, u64>,
}

impl UsageTracker {
    pub fn new(
        day: NaiveDate,
        today: HashMap<String, u64>,
        yesterday: HashMap<String, u64>,
    ) -> Self {
        Self {
            day,
            today,
            yesterday,
        }
    }

    pub fn day(&self) -> NaiveDate {
        self.day
    }

    pub fn today(&self) -> &HashMap<String, u64> {
        &self.today
    }

    /// 记一次前台采样：若 `at` 晚于当前记录的日期，先把 `today` 滚动为
    /// `yesterday` 再计数；`process` 为 `None` 表示本次未采到前台进程
    /// （只滚动日期，不累加任何进程）。
    pub fn record(&mut self, at: NaiveDate, process: Option<&str>, elapsed: Duration) {
        self.roll_to(at);
        if let Some(name) = process {
            *self.today.entry(name.to_string()).or_insert(0) += elapsed.as_secs();
        }
    }

    /// 跨日滚动：`at` 晚于当前日期时，把 `today` 平移为 `yesterday`（无论
    /// 中间跳过了多少天——daemon 停跑期间没有数据可言，"上一次有数据的一天"
    /// 就是当前语境下最合理的"昨日"），清空 `today` 并把日期推进到 `at`。
    /// `at` 早于或等于当前日期（时钟回拨 / 同一天内的重复调用）时不做任何事。
    fn roll_to(&mut self, at: NaiveDate) -> bool {
        if at <= self.day {
            return false;
        }
        self.yesterday = std::mem::take(&mut self.today);
        self.day = at;
        true
    }

    /// 今日 Top N（按时长降序，并列按进程名排序保证结果稳定）+ 对应的昨日
    /// 时长（无昨日记录则为 0）。
    pub fn snapshot(&self, ts_ms: u64) -> AppUsageSnapshot {
        let mut top: Vec<AppUsageEntry> = self
            .today
            .iter()
            .map(|(name, &today_secs)| AppUsageEntry {
                name: name.clone(),
                today_secs,
                yesterday_secs: self.yesterday.get(name).copied().unwrap_or(0),
            })
            .collect();
        top.sort_by(|a, b| {
            b.today_secs
                .cmp(&a.today_secs)
                .then_with(|| a.name.cmp(&b.name))
        });
        top.truncate(TOP_N);
        AppUsageSnapshot { ts_ms, top }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn accumulates_same_process_across_ticks() {
        let mut tracker = UsageTracker::new(date(2026, 8, 11), HashMap::new(), HashMap::new());
        tracker.record(date(2026, 8, 11), Some("firefox"), Duration::from_secs(5));
        tracker.record(date(2026, 8, 11), Some("firefox"), Duration::from_secs(5));
        tracker.record(date(2026, 8, 11), Some("code"), Duration::from_secs(5));

        assert_eq!(tracker.today().get("firefox"), Some(&10));
        assert_eq!(tracker.today().get("code"), Some(&5));
    }

    #[test]
    fn none_process_does_not_add_entries() {
        let mut tracker = UsageTracker::new(date(2026, 8, 11), HashMap::new(), HashMap::new());
        tracker.record(date(2026, 8, 11), None, Duration::from_secs(5));
        assert!(tracker.today().is_empty());
    }

    #[test]
    fn rolls_today_into_yesterday_on_date_change() {
        let mut today = HashMap::new();
        today.insert("firefox".to_string(), 120u64);
        let mut tracker = UsageTracker::new(date(2026, 8, 11), today, HashMap::new());

        tracker.record(date(2026, 8, 12), Some("code"), Duration::from_secs(5));

        assert_eq!(tracker.day(), date(2026, 8, 12));
        assert_eq!(tracker.today().get("code"), Some(&5));
        assert_eq!(tracker.today().get("firefox"), None);

        let snap = tracker.snapshot(0);
        let code_entry = snap.top.iter().find(|e| e.name == "code").unwrap();
        assert_eq!(code_entry.yesterday_secs, 0); // 昨日没有 "code" 的记录
    }

    #[test]
    fn rolls_across_multi_day_gap_using_last_known_day_as_yesterday() {
        let mut today = HashMap::new();
        today.insert("firefox".to_string(), 120u64);
        let mut tracker = UsageTracker::new(date(2026, 8, 8), today, HashMap::new());

        // daemon 停跑了几天，直接跳到 8/11。
        tracker.record(date(2026, 8, 11), Some("code"), Duration::from_secs(5));

        assert_eq!(tracker.day(), date(2026, 8, 11));
        let snap = tracker.snapshot(0);
        let code_entry = snap.top.iter().find(|e| e.name == "code").unwrap();
        // "昨日" 语义上就是"上一次有数据的那天"，即便实际间隔了 3 天。
        assert_eq!(code_entry.today_secs, 5);
    }

    #[test]
    fn ignores_clock_rollback() {
        let mut tracker = UsageTracker::new(date(2026, 8, 11), HashMap::new(), HashMap::new());
        tracker.record(date(2026, 8, 11), Some("firefox"), Duration::from_secs(5));

        let rolled = tracker.roll_to(date(2026, 8, 10));

        assert!(!rolled);
        assert_eq!(tracker.day(), date(2026, 8, 11));
        assert_eq!(tracker.today().get("firefox"), Some(&5));
    }

    #[test]
    fn snapshot_top_n_sorted_desc_with_yesterday_compare() {
        let mut today = HashMap::new();
        today.insert("a".into(), 10);
        today.insert("b".into(), 30);
        today.insert("c".into(), 20);
        today.insert("d".into(), 5);
        today.insert("e".into(), 1);
        today.insert("f".into(), 50);
        let mut yesterday = HashMap::new();
        yesterday.insert("b".into(), 15);
        let tracker = UsageTracker::new(date(2026, 8, 11), today, yesterday);

        let snap = tracker.snapshot(123);

        assert_eq!(snap.ts_ms, 123);
        assert_eq!(snap.top.len(), 5); // 6 个进程只取前 5
        assert_eq!(snap.top[0].name, "f");
        assert_eq!(snap.top[0].today_secs, 50);
        assert_eq!(snap.top[1].name, "b");
        assert_eq!(snap.top[1].yesterday_secs, 15);
        assert!(!snap.top.iter().any(|e| e.name == "e")); // 最小的被截断
    }

    #[test]
    fn snapshot_tie_break_is_deterministic_by_name() {
        let mut today = HashMap::new();
        today.insert("zeta".into(), 10);
        today.insert("alpha".into(), 10);
        let tracker = UsageTracker::new(date(2026, 8, 11), today, HashMap::new());

        let snap = tracker.snapshot(0);

        assert_eq!(snap.top[0].name, "alpha");
        assert_eq!(snap.top[1].name, "zeta");
    }
}
