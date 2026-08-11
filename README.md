# auxdeck

HYTE Y70 Touch Infinite 机箱副屏的常驻应用，替代 HYTE Nexus。项目全部口径与约束见 [CLAUDE.md](CLAUDE.md)。

## 结构

```
crates/auxdeck-core      跨进程共享类型（serde 契约：SystemSnapshot / ShellConfig / Push …）
crates/auxdeck-daemon    采集 collectors + config 热重载 + WebSocket RPC（127.0.0.1:9600）
crates/auxdeck-hostage   外部窗口拖放停靠 / 托管（M3b）
shell/                Tauri v2 + React 渲染层（网格布局 + 多页）
```

## 环境先决条件

- Rust stable 工具链
- Node.js（shell / Tauri v2）
- WebView2 Runtime（Windows 11 自带）

## 运行

**日常直跑（推荐）**——构建后一条命令，daemon 自动拉起并守护同目录的 shell：

```
cargo build --release -p auxdeck-daemon
cd shell; npm run tauri build; cd ..
target\release\auxdeck-daemon.exe --console
```

**前端开发迭代**（vite HMR）：

```
cargo run -p auxdeck-daemon -- --console   # 终端 1
cd shell; npm run tauri dev             # 终端 2
```

注意（重要）：**shell 必须经 tauri CLI 构建**。Tauri v2 判定 production 的依据是
`custom-protocol` feature（`tauri build` 自动启用）而非 build profile——纯
`cargo build --release -p auxdeck-shell` 或 `cargo run -p auxdeck-shell` 产出的都是
dev 形态、启动即连 vite devUrl，没有 dev server 时显示「localhost 拒绝连接」。

daemon 默认日志写 `%APPDATA%\auxdeck\logs\`；配置文件在 `%APPDATA%\auxdeck\config.toml`（首次运行自动生成，修改后热重载、无需重启）。

## CPU 温度：LibreHardwareMonitor（必读，按最小化配置）

AMD CPU 温度不走内核驱动，由 LHM 提供（CLAUDE.md §4）。未安装/未运行时 CPU 温度显示 N/A，其余功能不受影响。**口径：最小化克制利用**——auxdeck 只从 LHM 取 CPU 温度这一个值，LHM 自身也裁到只监控 CPU。

1. 下载 [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases) 并解压
2. **裁剪硬件类别**（减负关键）：菜单 Options → 取消勾选 GPU、Storage、Network、
   Mainboard/SuperIO、Memory、Controller 等一切非 CPU 类别，只留 **CPU**
3. 开启数据端点：Options → **Remote Web Server → Run**（默认端口 8085，仅本机访问；
   daemon 轮询 `http://localhost:8085/data.json`）
4. Options → **Minimize To Tray**，最小化到托盘常驻
5. 设置开机自启（读 CPU 温度需要管理员权限）：任务计划程序 → 创建任务 →
   「使用最高权限运行」、触发器「登录时」、操作指向 LibreHardwareMonitor.exe
6. 代价说明：全功能 LHM 常驻约 50–150MB；按上述裁剪后足迹显著更小——这是
   「不写 ring-0 驱动」的既定交换条件

## 天气配置

编辑 `%APPDATA%\auxdeck\config.toml`：

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

## License

MIT OR Apache-2.0，任选其一（[LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE)）。
除非明确声明，你有意提交给本项目的任何贡献均按上述双许可授权，无附加条款。
