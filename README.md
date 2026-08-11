# HyteDeck

HYTE Y70 Touch Infinite 机箱副屏的常驻应用，替代 HYTE Nexus。项目全部口径与约束见 [CLAUDE.md](CLAUDE.md)。

## 结构

```
crates/hyte-core      跨进程共享类型（serde 契约：SystemSnapshot / ShellConfig / Push …）
crates/hyte-daemon    采集 collectors + config 热重载 + WebSocket RPC（127.0.0.1:9600）
crates/hyte-hostage   外部窗口拖放停靠 / 托管（M3b）
shell/                Tauri v2 + React 渲染层（网格布局 + 多页）
```

## 环境先决条件

- Rust stable 工具链
- Node.js（shell / Tauri v2）
- WebView2 Runtime（Windows 11 自带）

## 运行（开发模式）

```
cargo run -p hyte-daemon -- --console   # 终端 1：采集 + WS 推送（--console 附加控制台日志）
cargo run -p hyte-shell                 # 终端 2：识别 682×2560 副屏并钉屏；找不到则落主屏小窗
```

daemon 默认日志写 `%APPDATA%\HyteDeck\logs\`；配置文件在 `%APPDATA%\HyteDeck\config.toml`（首次运行自动生成，修改后热重载、无需重启）。

## CPU 温度：LibreHardwareMonitor（必读）

AMD CPU 温度不走内核驱动，由 LHM 提供（CLAUDE.md §4）。未安装/未运行时 CPU 温度显示 N/A，其余功能不受影响。

1. 下载 [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases) 并解压
2. 运行后勾选 Options → **Remote Web Server → Run**（默认端口 8085，daemon 轮询 `http://localhost:8085/data.json`）
3. 设置开机自启（需要管理员权限才能读到 CPU 温度）：任务计划程序 → 创建任务 →
   「使用最高权限运行」、触发器「登录时」、操作指向 LibreHardwareMonitor.exe
4. 代价说明：LHM 常驻约 50–150MB 内存——这是「不写 ring-0 驱动」的既定交换条件

## 天气配置

编辑 `%APPDATA%\HyteDeck\config.toml`：

```toml
[weather]
enabled = true
provider = "open-meteo"   # 免 key；或 "qweather"（需 qweather_key）
latitude = 31.23          # 按实际位置填写
longitude = 121.47
city_label = "上海"
```

保存即热生效。不做 IP 定位（CLAUDE.md §7.1）。

## 提交门槛

`cargo fmt --check` · `cargo clippy -- -D warnings` · `cargo test`
