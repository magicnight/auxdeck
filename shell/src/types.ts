// daemon <-> shell 消息契约。形状必须与 crates/hyte-core/src/lib.rs 的
// SystemSnapshot / Push 保持一致，不得单方面改动（CLAUDE.md §10）。

export interface SystemSnapshot {
  ts_ms: number;
  cpu_usage: number;
  cpu_temp: number | null;
  gpu_usage: number | null;
  gpu_temp: number | null;
}

export interface SystemPush {
  type: "system";
  data: SystemSnapshot;
}
