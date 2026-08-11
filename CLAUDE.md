# CLAUDE.md — HyteDeck

给 Claude Code 的项目上下文。开始任何工作前先读完本文件。

---

## 1. 项目定位

为 HYTE Y70 Touch Infinite 机箱内置副屏编写一个替代 HYTE Nexus 的常驻应用（工作代号 **HyteDeck**）。

目标：小组件面板（时钟 / 闹钟 / 天气 / 日程 / 系统信息 / 应用使用时长 / 背景动画）+ 视频与网页内容托管 + 外部窗口拖放停靠 + 触摸优先交互。核心诉求是**低常驻资源占用**与**可自定义**，Nexus 的问题是 Electron 的内存与空闲 CPU 开销，不是磁盘体积。

**量化性能目标（验收依据）：**

| 项 | 目标 |
|---|---|
| hyte-daemon | 空闲 CPU < 1%（含 1s 采样与 LHM 轮询）、内存 < 30MB |
| hyte-shell | 无动画时 CPU ≈ 0%；WebView2 进程组内存 ≤ 250MB |
| 背景动画 | 非游戏时 GPU < 5%；游戏时按 §9.4 降级 |

预期对齐：WebView2 自身 100–200MB 不可避免。本项目赢在空闲 CPU、采集开销与进程数量，不承诺把渲染层内存降到个位数。动工前先实测 Nexus 基线，记入 §12。

**长期愿景（v2+，不挤占 M1–M4 主线）**：演进为通用「副屏助手」——支持任意副屏（不限 HYTE 面板）显示数据组件；公布 widget 组件接口，接受社区提交。当前架构已为此预留：widget 以 kind 字符串开放注册（未知 kind 前向兼容不崩）、第三方内容一律 iframe/webview 沙箱（§10）、选屏机制可通用化（§8.4）。涉及插件协议的架构决策（组件形态、manifest、分发）在 M6 前专项定夺。

---

## 2. 硬件事实（已核实，不要重新推测）

| 项 | 值 |
|---|---|
| 面板 | 14.9" IPS，682 × 2560，60Hz，500 nits，178 PPI |
| 触摸 | 10 点电容多点触控，标准 USB HID digitizer |
| 接口 | DisplayPort（视频）+ USB 2.0 针脚（触摸）+ SATA（供电） |
| 系统呈现 | **一台普通的第二显示器**，非私有协议设备 |
| 当前设置 | Windows 显示方向设为「纵向（翻转）」 |

**最重要的一条：不需要逆向任何 Nexus 私有显示协议。** 本项目的本质是"钉在指定显示器上的无边框常驻窗口 + 该显示器的窗口管理器"。

例外：机箱 LED 灯效走私有 USB 指令，若后续要接管需单独逆向。可参考 `YanissAmz/hyte-y70-touch-dashboard`（Linux，已逆向约 700 条 LED 指令）。

---

## 3. 架构（三层分离 + 进程模型）

```
hyte-daemon        Rust 常驻服务，无 UI
  ├─ collectors/   sysinfo · nvml · lhm · weather · calendar · ai-usage · app-usage
  ├─ state/        聚合、采样节流、历史环形缓冲
  ├─ alarm/        闹钟触发与播声（shell 崩了也要响，§7.3）
  └─ rpc/          127.0.0.1 上的 WebSocket 推送 + JSON-RPC

hyte-shell         渲染层，无边框常驻窗口 @ 副屏
  ├─ 4 列 × n 行网格 + 多页（§8.1），widget 按格子占位
  └─ 订阅 daemon 推送，自身不做任何采集

hyte-hostage       外部窗口托管
  ├─ 拖放停靠：全局监听窗口拖动 → 网格吸附 → 全屏压回停靠区（§6.2）
  └─ `--app=` / kiosk 拉起、z-order 巡逻与焦点管理
```

分层的目的是渲染层可替换。**collector 逻辑绝不允许写进 shell。**

**进程模型：**

- 开机自启只注册一条：任务计划（登录触发、普通权限、失败自动重启）启动 hyte-daemon
- daemon 兼任 supervisor：启动并守护 shell 与 hostage，崩溃后指数退避重启
- daemon 失联时 shell 显示「重连中」降级 UI 并持续重连，shell 不自杀；失联超 30s 反向拉起 daemon（互为看门狗，named mutex + 退避防拉起风暴）
- 各进程 named mutex 单实例
- 检测到 Nexus 进程时不强退，仅在 shell 显示常驻警告条（副屏与 LED 会被两家争抢）
- M1–M4 不做自动更新，手动替换二进制

---

## 4. 技术栈决策

**已选定：**

- Rust workspace：`hyte-core`（共享类型）+ `hyte-daemon` + `hyte-hostage`；`hyte-shell` 为 Tauri v2 项目，其 src-tauri crate 加入同一 workspace
- shell 层用 **Tauri v2 + React**，理由：WebView2 系统自带、产物 3–6MB、682px 竖屏网格 UI 用 CSS 迭代最快、网页类小组件可直接内嵌
- 系统数据：`sysinfo`（CPU/内存/磁盘/网络）+ `nvml-wrapper`（RTX 5090 温度/功耗/显存/利用率）
- AMD CPU 温度：**不写 ring-0 驱动**。轮询 LibreHardwareMonitor 的 HTTP server `http://localhost:8085/data.json`
  - daemon 不提权、不负责拉起 LHM；LHM 由用户配置为计划任务「登录时最高权限自启」（README 写清步骤）
  - LHM 不可用 → CPU 温度显示 N/A，其余 collector 不受影响
  - 接受的代价：LHM 常驻约 50–150MB、需管理员权限——这是「不写驱动」的交换条件
  - **无 Rust 等价库**（已核实勿再找）：Windows 用户态读不到 AMD CPU 温度，必须 ring-0 访问 MSR/SMN 寄存器——任何语言的方案都得带内核驱动，LHM 的价值正是它自带的签名驱动；WMI 的 `MSAcpi_ThermalZoneTemperature` 是主板 ACPI 温区、非 CPU die 温度且多数主板不更新，不采用
  - **最小化克制利用口径（2026-08-11 拍板）**：①数据面——LHM 只作为 CPU 温度单一值的来源，其余传感器一律不从 LHM 走（GPU 走 NVML、系统指标走 sysinfo）；②LHM 配置面——只启用 CPU 硬件类别监控（GPU/存储/网络/主板全部取消勾选），Web server 仅本机 8085，最小化到托盘（步骤见 README）；③代码面——不可用时 30s 退避探测、全进程单 HTTP client、可用性状态变化才记日志、失败完全隔离
- RPC：WebSocket 仅绑 127.0.0.1，v1 不加 token；RPC 永不暴露「执行任意命令 / 打开任意文件」类接口；后续引入敏感控制（LED、hostage 指令）时再补 token

**已评估并排除（不要再提议）：**

- Electron / Node 运行时 —— 正是要替换掉的东西
- 纯 Rust GPU UI（Slint、egui + wgpu）—— 体积更小但放弃全部网页内容能力；若后续要极致轻量版本可作为 shell 的第二实现，不作为首选
- 自写内核态传感器驱动 —— 收益不抵风险
- 逆向 Nexus 显示协议 —— 不必要，见 §2

**Cargo release profile：**

```toml
[profile.release]
opt-level = "z"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

---

## 5. 硬约束（会卡住需求的事实，实现前务必确认口径）

**5.1 DRM 视频**

- WebView2 不携带 Widevine / PlayReady CDM → Netflix 等 DRM 站点在内嵌 WebView 中**必定失败**
- Windows Graphics Capture 对受 DRM 保护的窗口返回黑屏 → 「抓别的浏览器窗口再投过来」同样不可行
- **唯一可行路径**：真实浏览器窗口放到副屏（Edge 走 PlayReady 可上 1080p+；Chrome 走 Widevine L3 上限 720p），实现形态见 §6.2 拖放停靠。窗口管理首选 `SetWindowPos` + z-order；`SetParent` 收编仅作实验性方案，出问题即放弃
- YouTube / Bilibili 在内嵌 WebView2 中通常可播（→ §6.1 播放卡），但 YouTube 部分内容走 EME，可能降码率或失败——需实测，别假设

**5.2 AI 订阅配额**

- Anthropic 与 OpenAI 均**无面向消费订阅额度的公开 API**
- 唯一实现路径：统计本地 `~/.claude/projects/*.jsonl` 得 Claude Code 用量（`ccusage` 的做法）。**增量解析、按文件记 offset，禁止每轮全量重扫**（本机 jsonl 持续膨胀）
- OpenAI 订阅侧无本地数据可读，不做；Admin/Usage API 需组织级 key，不适用；爬网页拿配额脆弱且可能违反 ToS，**不实现**

---

## 6. 内容托管设计

**6.1 内嵌视频播放卡（M3a）**

- YouTube / B站走 shell 内嵌 WebView2 播放卡；hostage 只留给 DRM 站点与任意外部窗口
- 无框界面：YouTube 用 IFrame embed（`youtube.com/embed/{videoId}`，`fs=0` 禁用原生全屏按钮）；B站用官方 embed（`player.bilibili.com/player.html?bvid=…`，自带弹幕开关）；播放器控件之外不出现任何站点导航 / 推荐流
- **容器内全屏**：「全屏」= 视频画面撑满该 widget 在网格上占据的容器区域；卡片不扩张、不覆盖其他 widget、不触发 OS 级全屏
  - 画面适配两模式：**fit**（默认，完整画面短边留黑）↔ **cover**（撑满，放大居中裁切无黑边）；「撑满」按钮即切换
  - cover 实现：跨域 iframe 内部样式不可控，用 oversize iframe + 容器 `overflow: hidden` 居中裁切
  - 站点绕过 `fs=0` 触发原生全屏时（WebView2 `ContainsFullScreenElementChanged`）一律拦截，映射为容器内 cover——原生全屏永不放行
  - 想要更大观看区域 = 去 §8.1 网格把格子改大，不是全屏机制的职责
- Shorts / 竖屏短视频：9:16 天然适配竖长容器，cover 撑满几乎无裁切；单条从 shorts URL 提取 videoId 走 embed；**滑动换条 feed** 需 mobile-UA 内嵌移动站（m.youtube.com/shorts、B站竖屏 feed），M3a 实测项，纯滑动无需打字
- 登录态：webview cookie 持久化，一次登录长期有效；B站登录后解锁高清晰度。实测风险：Google 可能拦截内嵌 webview 登录（"browser not secure"），被拦则未登录 embed 播放，或该站回退 §6.2 停靠
- WebView2 启动参数放开自动播放：`--autoplay-policy=no-user-gesture-required`
- 选片交互：M3a 先做主屏推送（剪贴板监听视频链接弹「在副屏播放」+ config 播放列表，不需副屏打字）；副屏站内浏览/搜索等 §9.1 输入方案验证通过后再加；shorts feed 不需打字可同期
- 游戏共存：视频硬解走 NVDEC 独立解码单元，不随 §9.4 动画降级而暂停；提供一键暂停

**6.2 拖放停靠 + 区域内全屏（M3b，hostage 核心形态）**

- 交互：把任意窗口（Edge / Chrome / mpv / 播放器…）拖入副屏 → shell 显示网格停靠高亮（drop zone）→ 松手吸附到停靠区（网格上划定的 w×h 区域）；停靠区标记 occupied，widget 不排入
- 实现（零注入，PowerToys FancyZones 同款路径）：`SetWinEventHook`（`EVENT_SYSTEM_MOVESIZESTART/END` + `EVENT_OBJECT_LOCATIONCHANGE`）监听窗口拖动，松手落在副屏 → `SetWindowPos` 吸附；z-order 由 hostage 巡逻（停靠窗在 shell 之上）
- **区域内全屏**：停靠窗按 F11 / 网页全屏试图铺满副屏时，hostage 检测「全屏意图」（窗口 rect == 显示器 rect 且无 `WS_CAPTION`）后**立即压回停靠区 rect**
  - 效果：浏览器认为自己在全屏（隐藏全部 chrome），实际被钉在停靠区 → **F11 即无框化**；真实 Edge → Netflix / PlayReady DRM 通路无损
  - 压回后 rect ≠ 显示器 rect，判定不再命中，无死循环；持续监听 + 节流防浏览器重扩；记录进全屏前的停靠 rect，退出全屏时矫正恢复
- M3b 实测清单：压回瞬间闪烁程度、Edge 重扩行为、退出全屏位置恢复、停靠窗触摸滚动表现
- 焦点口径：停靠窗是用户主动引入的普通窗口，点击获得焦点属预期（与 shell 的 NOACTIVATE 无冲突）；可选实验项：「游戏模式」对停靠窗附加 `WS_EX_NOACTIVATE`（能点、能滚、不能打字、不切出游戏）
- `--app=` 主动拉起后做，复用同一套停靠区管理与压回逻辑

---

## 7. widget 数据源口径

**7.1 天气**：collector 做成 provider trait 可替换；默认 QWeather（和风天气，国内稳定、中文天况文案，免费 key 放 config），备选 Open-Meteo（免 key 免注册）；位置手动配置（城市名或经纬度），**不做 IP 定位**；日出日落用本地系统时区换算

**7.2 日程**：只接两种源——iCloud CalDAV（app-specific password 放 config）+ 通用 ICS 订阅 URL；不做 Google / Outlook OAuth 流程

**7.3 闹钟**：触发与播声在 daemon（系统默认音频设备出声，副屏无扬声器；shell 崩了也要响），同时推事件给 shell 显示全屏闹钟卡（触摸停止/贪睡）；PC 睡眠/关机时不响、不配置唤醒定时器——定位是「人在电脑旁时的提醒」，不替代手机闹钟

**7.4 应用使用时长**：daemon 每 5s 轮询前台窗口所属进程，累计各进程当日前台时长，按日持久化本地文件；widget 展示今日 Top 应用 + 与昨日对比；纯本地统计，无外发

**7.5 AI 用量**：见 §5.2，只统计 Claude Code

---

## 8. 布局、自定义与首次运行

**8.1 网格布局（对齐 Nexus 模型）**：**4 列 × n 行网格 + 多页**（682px ÷ 4 列 ≈ 170px/列）。每个 widget 声明网格坐标 `(page, x, y, w, h)`；页内网格排布，页间上下切换（保留分页指示点）；网格参数（行高、间距、页数）进 config，默认观感对齐 Nexus

**8.2 v1 自定义边界**：config.toml + 热重载可改——widget 启用与网格坐标、主题色、动画开关、数据源与 key、采样频率。不做屏上编辑器、不做自由像素级拖拽

**8.3 副屏不承载管理 UI**：不复刻 Nexus 底部 dock（文件夹/媒体库）。682px 屏只放 widget 与内容；配置 GUI 做主屏小窗，已排 M5

**8.4 首次运行选屏**：daemon 枚举显示器，发现 682×2560 面板即认定为目标屏，写 EDID/设备实例路径入 config；未发现则 shell 落主屏并显示一次性选屏列表

---

## 9. Windows 侧已知坑（实现时逐条对照）

1. **不能抢焦点**：shell 常态 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`（Tauri 需下沉 `windows-rs` 手动设置），否则游戏中点一下屏幕就切出去。需要文本输入的场景（网页登录、搜索）临时移除 NOACTIVATE 允许激活，结束后恢复。**M1 首要验证**：NOACTIVATE 下 WebView2 触摸滚动/点击是否正常、触摸键盘是否因无焦点不弹。若触摸不可用，交互设计回炉——全项目最大单点风险，最先做。
2. **触摸映射**：屏幕旋转后 digitizer 坐标常错乱。这是 Windows 层的一次性校准（平板电脑设置 → 设置），**不是代码问题**，不要试图在应用里补偿。
3. **显示器重枚举**：待机唤醒、全屏独占游戏切换会触发 `WM_DISPLAYCHANGE`，窗口被踢回主屏。必须监听并**按 EDID / 设备实例路径重新定位**，禁止硬编码坐标。
4. **动画降级**：检测到前台为全屏游戏时，背景动画降到 15–30fps 或暂停，不与游戏抢 GPU。
5. **UI 范式**：682px 有效宽度 ≈ 一台手机竖屏。按移动端网格 widget 设计（§8.1），大触摸热区，**不要套桌面布局**。
6. **DPI 基准**：副屏 Windows 缩放固定 100%，设计稿按 682×2560 @1x CSS px；shell 声明 per-monitor v2 DPI aware；UI 大小用 CSS 变量调，不靠系统缩放。
7. **手势仲裁**：页面上下切换只响应「屏幕右缘 24px 手势带」；widget 内部（含内嵌网页滚动、shorts 上滑换条）的手势全部交给 widget 自身。备选：底部圆点点按翻页。

---

## 10. 开发约定

- 提交前必须通过 `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test`
- collector 一律实现统一 trait，可单独禁用；任一 collector 失败不得影响其余数据推送
- 所有跨进程数据结构定义在 `hyte-core`，serde 序列化
- 采样频率可配置，默认：系统指标 1s、天气 10min、日程 5min、AI 用量 5min、前台应用 5s
- 配置文件用 TOML，放 `%APPDATA%\HyteDeck\config.toml`
- 日志用 `tracing`，默认写文件不写 stdout（无控制台常驻进程）
- 第三方网页一律装入独立 webview/iframe，不注入任何 Tauri IPC；只有本地 UI bundle 拥有 IPC capability

---

## 11. 里程碑

**M1（先做这个，其余暂不动）**
daemon 起 WebSocket → shell 钉在副屏且不抢焦点 → 显示时钟 + CPU/GPU 温度利用率。

M1 验收标准：

1. 全屏游戏中触摸副屏，游戏不失焦、不切出
2. 连续运行 24h：daemon < 30MB 且无增长趋势，shell 无泄漏式增长
3. 待机唤醒 / 拔插显示器 / 独占全屏切换后，窗口 5s 内回到副屏正确位置
4. kill daemon → shell 显示重连态并自动恢复；kill shell → daemon 自动拉起
5. 关闭 LHM 时 CPU 温度显示 N/A，其余数据不断流

**M2** 网格布局框架 + 分页切换 + 天气 / 闹钟 / 日程 / 应用使用时长 widget + 配置文件热重载。

**M3a** 内嵌视频播放卡（§6.1）：YouTube/B站 embed、容器内全屏 fit/cover、shorts、主屏推送选片。

**M3b** 拖放停靠（§6.2）：网格吸附、区域内全屏压回、DRM 通路（Netflix）；`--app=` 主动拉起后做。

**M4** tray 常驻（Tauri tray API：显示/隐藏 shell、重启、打开设置、退出）、背景动画引擎（含像素宠物类前景精灵，是否做见 §13-Q5）、AI 用量 widget、LED 灯效接管（**可选，可砍不影响主线**；需先逆向，参考 §2 的外部项目）。

**M5** 设置页面（主屏小窗 GUI）：各组件的数据源、网格排列、大小、背景等可视化编辑，写回 config.toml 复用热重载通路；副屏仍不承载管理 UI（§8.3）。

**M6（v2 愿景）** 通用副屏助手：任意副屏选择与多屏适配、widget 插件接口规范化并公布文档、社区组件提交与分发机制（形态候选：iframe/web 组件沙箱包，安全边界沿用 §10）。

---

## 12. Nexus 现状基线（parity 对照）

依据 2026-08-10 副屏截图（第 1 页，共 2 页）：

| Nexus 现有元素 | 说明 | HyteDeck 对应 |
|---|---|---|
| 时钟卡 | 时间 + 中文日期 | M1 |
| 应用使用时长卡 | 「Firefox 今天已使用 42 分 37 秒，相比昨日提升超过两倍」 | §7.4，M2 |
| 像素宠物 + 粒子波浪背景 | 前景精灵动画 + 背景动画，两层 | M4 |
| 天气卡 | 当前温 / 天况 / 高低温 / 城市 / 日出日落 | §7.1，M2 |
| 硬件仪表卡 | 核心#1 VID 0.37V、GPU 3%、CPU 核心#1 31%、GPU 37° | M1；用聚合指标，不照抄单核 VID 类零散指标 |
| 分页指示（2 页） | 上下分页 | §8.1 |
| 底部 dock（文件夹 / 媒体库 / 空位） | Nexus 的媒体管理入口 | 不做（§8.3） |
| 布局系统 | 组件尺寸按 n×4 网格设定 | §8.1 采纳同型模型 |

截图暴露的 Nexus 问题（引以为戒）：天气定位显示 Los Angeles、日落时间「下午4:11」异常 → §7.1 要求位置手动配置、时区换算正确；硬件卡无 CPU 温度 → LHM 方案的意义（§4）。

**资源基线（动工前实测填入）：**

| 指标 | Nexus 实测（2026-08-10） | HyteDeck 目标 |
|---|---|---|
| 进程数 | 7（含 HYTE.Nexus.Service） | 3（+WebView2 子进程） |
| 总内存 | 366.8 MB | daemon <30MB + shell ≤250MB |
| 空闲 CPU | 1.27%（4s 采样窗口） | <1% |
| GPU 占用 | 未测 | 动画时 <5% |

---

## 13. 待定问题（默认口径先行，回答后更新对应章节）

| # | 问题 | 未答时的默认口径 |
|---|---|---|
| Q1 | 副屏第 2 页截图（parity 清单可能不全） | 以第 1 页清单为准 |
| Q2 | 日程主力是 iCloud 日历（me.com）吗 | 按 §7.2 iCloud CalDAV + ICS 实现 |
| Q3 | 天气 provider：QWeather 还是 Open-Meteo | 默认 QWeather（需用户注册免费 key） |
| Q4 | app-usage 使用时长卡要不要 | 做，排 M2 尾（§7.4） |
| Q5 | 像素宠物动画要不要 parity | M4 动画引擎预留前景精灵层，做否届时定 |
| Q6 | 天气显示哪个城市 | M2 配置时提供；不做 IP 定位 |
| Q7 | 选片交互「主屏推 URL + 剪贴板监听」是否接受 | 按 §6.1 实现，剪贴板识别纯本地 |
| Q8 | daemon 内存目标口径：release 实测 WorkingSet 42.6MB（其中 nvml.dll 映射即占 22MB，GPU 采集固定成本）/ Private 30.8MB | 暂按 Private ≤ 32MB 执行；§1 的 30MB 是否改为 Private 口径或 WS ≤ 45MB 待定夺 |
