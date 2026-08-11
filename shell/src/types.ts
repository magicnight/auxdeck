// daemon <-> shell 消息契约。形状必须与 crates/hyte-core/src/lib.rs 的
// SystemSnapshot / WeatherSnapshot / AppUsageSnapshot / ShellConfig / Push 保持一致，
// 不得单方面改动（CLAUDE.md §10）。

export interface SystemSnapshot {
  ts_ms: number;
  cpu_usage: number;
  cpu_temp: number | null;
  gpu_usage: number | null;
  gpu_temp: number | null;
}

export interface WeatherSnapshot {
  ts_ms: number;
  city: string;
  temp_c: number;
  condition: string;
  high_c: number;
  low_c: number;
  sunrise: string;
  sunset: string;
}

export interface AppUsageEntry {
  name: string;
  today_secs: number;
  yesterday_secs: number;
}

export interface AppUsageSnapshot {
  ts_ms: number;
  top: AppUsageEntry[];
}

/** 网格布局参数（CLAUDE.md §8.1）。 */
export interface GridConfig {
  columns: number;
  row_height_px: number;
  gap_px: number;
}

/** 单个 widget 的网格占位 `(page, x, y, w, h)`。kind 未知时 shell 忽略该 widget。 */
export interface WidgetPlacement {
  kind: string;
  page: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ShellConfig {
  grid: GridConfig;
  widgets: WidgetPlacement[];
}

export interface SystemPush {
  type: "system";
  data: SystemSnapshot;
}

export interface WeatherPush {
  type: "weather";
  data: WeatherSnapshot;
}

export interface AppUsagePush {
  type: "app_usage";
  data: AppUsageSnapshot;
}

export interface ConfigPush {
  type: "config";
  data: ShellConfig;
}

/** daemon -> shell 的 WebSocket 推送信封（对应 hyte_core::Push 的 4 个变体）。 */
export type Push = SystemPush | WeatherPush | AppUsagePush | ConfigPush;

/**
 * 与 `hyte_core::ShellConfig::default()` 同形的内置默认布局（CLAUDE.md §12 对齐
 * Nexus 第 1 页观感）。daemon 尚未推送 Config 时用它渲染，避免空屏。
 * 若 Rust 侧默认值变化，需同步更新此常量。
 */
export const DEFAULT_SHELL_CONFIG: ShellConfig = {
  grid: { columns: 4, row_height_px: 150, gap_px: 12 },
  widgets: [
    { kind: "clock", page: 0, x: 0, y: 0, w: 4, h: 3 },
    { kind: "metrics", page: 0, x: 0, y: 3, w: 4, h: 4 },
    { kind: "weather", page: 0, x: 0, y: 7, w: 4, h: 4 },
    { kind: "app_usage", page: 0, x: 0, y: 11, w: 4, h: 4 },
  ],
};
