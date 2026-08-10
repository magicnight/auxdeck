# HyteDeck

HYTE Y70 Touch Infinite 机箱副屏的常驻应用，替代 HYTE Nexus。项目全部口径与约束见 [CLAUDE.md](CLAUDE.md)。

## 结构

```
crates/hyte-core      跨进程共享类型（serde）
crates/hyte-daemon    采集 collectors + WebSocket RPC
crates/hyte-hostage   外部窗口拖放停靠 / 托管
shell/                Tauri v2 + React 渲染层（M1 初始化）
```

## 环境先决条件

- Rust stable 工具链
- Node.js（shell / Tauri v2）
- WebView2 Runtime（Windows 11 自带）
- LibreHardwareMonitor：设为计划任务「登录时最高权限自启」，daemon 轮询 `http://localhost:8085/data.json` 取 CPU 温度；未运行时温度显示 N/A，其余功能不受影响

## 提交门槛

`cargo fmt --check` · `cargo clippy -- -D warnings` · `cargo test`
