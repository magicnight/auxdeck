//! hyte-shell：Tauri v2 渲染层（CLAUDE.md §3 / §9）。
//! M1：窗口钉在目标副屏、附加 NOACTIVATE 不抢焦点、每 5s 巡检位置漂移并搬回。
//! 找不到目标屏时退化为开发态：主屏、带边框、不设 NOACTIVATE（CLAUDE.md §9.1）。
//!
//! 定位一律走 Win32 `SetWindowPos`（物理像素、单次原子调用）：混合 DPI 环境
//! （本机主屏 150% / 副屏 100%）下，tauri 的 set_position + set_size 两步调用
//! 会与跨屏移动触发的 `WM_DPICHANGED` 自动调整竞争，实测产生左/上偏移露出桌面。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use tauri::{App, Manager, PhysicalSize, Size, WebviewWindow};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_BORDER_COLOR,
    DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
    SET_WINDOW_POS_FLAGS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_SYSMENU, WS_THICKFRAME,
};

/// 目标副屏物理像素尺寸（CLAUDE.md §2）。显示方向可能纵向或纵向翻转，宽高互换也算命中。
const PANEL_SIZE_A: (u32, u32) = (682, 2560);
const PANEL_SIZE_B: (u32, u32) = (2560, 682);

/// 找不到目标屏时的开发态兜底窗口尺寸。
const FALLBACK_WIDTH: u32 = 400;
const FALLBACK_HEIGHT: u32 = 800;

/// 位置巡检间隔，对应 M1 验收标准 3「5s 内回到副屏正确位置」。
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);

/// 目标副屏在虚拟桌面中的物理像素矩形，setup 阶段发现一次，巡检线程据此纠偏。
/// WM_DISPLAYCHANGE 子类化、按 EDID 重新枚举留待后续里程碑，M1 不做。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TargetRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn is_panel_monitor(size: &PhysicalSize<u32>) -> bool {
    let dims = (size.width, size.height);
    dims == PANEL_SIZE_A || dims == PANEL_SIZE_B
}

/// 附加 WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW：窗口可显示、可接收触摸，但永不抢占前台焦点（CLAUDE.md §9.1）。
fn apply_noactivate(hwnd: HWND) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let ex_bits = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current | ex_bits);
    }
}

/// 剥掉 WS_CAPTION / WS_THICKFRAME / WS_SYSMENU：tao 的无边框窗口保留这些
/// 样式位换取系统动画，代价是 DWM 按标准窗口给它布局不可见边框（实测左/右/下
/// 各 7px）。外扩补偿会让窗口左缘越进主屏（不同 DPI），跨屏合成产生 1px 舍入缝。
/// 对钉死副屏、永不最小化/最大化的常驻窗口，纯 popup 形态才是真全屏：
/// 无边框布局、无需补偿、窗口矩形不越界。样式变更后 SWP_FRAMECHANGED 强制重算。
fn strip_window_frame(hwnd: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let cleared = style & !((WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0) as isize);
        SetWindowLongPtrW(hwnd, GWL_STYLE, cleared);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SET_WINDOW_POS_FLAGS(
                SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOZORDER.0 | SWP_NOACTIVATE.0
                    | SWP_FRAMECHANGED.0,
            ),
        );
    }
}

/// 关掉 Win11 给窗口的装饰性描边：圆角（四角露桌面）与 1px 边框描边
/// （四周细缝）。钉满副屏的常驻窗口要的是「真全屏」，这些装饰只会漏出背景。
fn strip_dwm_adornments(hwnd: HWND) {
    unsafe {
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const core::ffi::c_void,
            std::mem::size_of_val(&corner) as u32,
        );
        // DWMWA_COLOR_NONE：完全不画边框描边。
        let no_border: u32 = 0xFFFF_FFFE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &no_border as *const _ as *const core::ffi::c_void,
            std::mem::size_of_val(&no_border) as u32,
        );
    }
}

/// 单次原子设置窗口位置+尺寸（物理像素），绕开 tauri 两步 API 与 DPI 变更的竞争。
fn set_window_rect(hwnd: HWND, rect: TargetRect) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            rect.x,
            rect.y,
            rect.width as i32,
            rect.height as i32,
            SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0),
        );
    }
}

/// 读回窗口当前物理矩形（GetWindowRect，无 DPI 换算歧义）。
fn window_rect(hwnd: HWND) -> Option<TargetRect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok()? };
    Some(rect_from(r))
}

/// DWM 实际渲染的可视边界。tao 无边框窗口保留 WS_CAPTION（借系统动画），
/// DWM 会给它算上不可见边框（实测本机左/右/下各 7px）：GetWindowRect 含边框、
/// 可视区更小，两侧露出桌面。
fn dwm_visible_rect(hwnd: HWND) -> Option<TargetRect> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?
    };
    Some(rect_from(r))
}

fn rect_from(r: RECT) -> TargetRect {
    TargetRect {
        x: r.left,
        y: r.top,
        width: (r.right - r.left).max(0) as u32,
        height: (r.bottom - r.top).max(0) as u32,
    }
}

/// 量出各边不可见边框厚度，返回「可视区恰好覆盖 panel」所需的窗口矩形。
/// 无边框差异时返回 None。
fn frame_compensated_rect(hwnd: HWND, panel: TargetRect) -> Option<TargetRect> {
    let wr = window_rect(hwnd)?;
    let vis = dwm_visible_rect(hwnd)?;
    let dl = vis.x - wr.x;
    let dt = vis.y - wr.y;
    let dr = (wr.x + wr.width as i32) - (vis.x + vis.width as i32);
    let db = (wr.y + wr.height as i32) - (vis.y + vis.height as i32);
    if dl == 0 && dt == 0 && dr == 0 && db == 0 {
        return None;
    }
    Some(TargetRect {
        x: panel.x - dl,
        y: panel.y - dt,
        width: (panel.width as i32 + dl + dr).max(1) as u32,
        height: (panel.height as i32 + dt + db).max(1) as u32,
    })
}

/// 生产路径：设样式 → 原子定位 → 显示 → 再钉一次（show 跨屏触发的 DPICHANGED
/// 可能让 tao 按 suggested rect 重调）→ 量 DWM 边框差值外扩补偿 → 验证可视区。
/// 返回补偿后的窗口矩形，供巡检线程作为纠偏目标。
fn pin_to_panel(window: &WebviewWindow, panel: TargetRect) -> tauri::Result<TargetRect> {
    let hwnd = window.hwnd()?;
    apply_noactivate(hwnd);
    strip_window_frame(hwnd);
    strip_dwm_adornments(hwnd);
    set_window_rect(hwnd, panel);
    window.show()?;
    set_window_rect(hwnd, panel);

    let target = frame_compensated_rect(hwnd, panel).unwrap_or(panel);
    if target != panel {
        set_window_rect(hwnd, target);
    }

    match dwm_visible_rect(hwnd) {
        Some(vis) if vis == panel => {
            eprintln!("[hyte-shell] pinned OK: visible {panel:?} (window rect {target:?})");
        }
        Some(vis) => {
            eprintln!("[hyte-shell] pin MISMATCH: want visible {panel:?}, got {vis:?} (window rect {target:?})");
        }
        None => eprintln!("[hyte-shell] pinned, DWM bounds unavailable (window rect {target:?})"),
    }
    Ok(target)
}

/// 开发态兜底：找不到目标屏时落主屏，带边框普通窗口，不设 NOACTIVATE。
fn show_fallback(window: &WebviewWindow) -> tauri::Result<()> {
    window.set_decorations(true)?;
    window.set_size(Size::Physical(PhysicalSize {
        width: FALLBACK_WIDTH,
        height: FALLBACK_HEIGHT,
    }))?;
    window.show()?;
    Ok(())
}

/// 每 5s 用 GetWindowRect 校验窗口仍精确铺满目标屏（待机唤醒 / 独占全屏切换
/// 踢回主屏、DPI 事件重调尺寸等），漂移则 SetWindowPos 原子复位。
fn spawn_position_watchdog(window: WebviewWindow, rect: TargetRect) {
    std::thread::spawn(move || loop {
        std::thread::sleep(WATCHDOG_INTERVAL);
        let Ok(hwnd) = window.hwnd() else {
            continue;
        };
        let Some(actual) = window_rect(hwnd) else {
            continue;
        };
        if actual != rect {
            eprintln!("[hyte-shell] drift: {actual:?} -> repin {rect:?}");
            set_window_rect(hwnd, rect);
        }
    });
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app
        .get_webview_window("main")
        .expect("tauri.conf.json 声明的 main 窗口必须存在");

    let mut target: Option<TargetRect> = None;
    for monitor in app.available_monitors()? {
        eprintln!(
            "[hyte-shell] monitor {:?}: pos=({}, {}) size={}x{} scale={}",
            monitor.name(),
            monitor.position().x,
            monitor.position().y,
            monitor.size().width,
            monitor.size().height,
            monitor.scale_factor(),
        );
        if target.is_none() && is_panel_monitor(monitor.size()) {
            target = Some(TargetRect {
                x: monitor.position().x,
                y: monitor.position().y,
                width: monitor.size().width,
                height: monitor.size().height,
            });
        }
    }

    match target {
        Some(rect) => {
            let pinned = pin_to_panel(&window, rect)?;
            spawn_position_watchdog(window.clone(), pinned);
        }
        None => {
            eprintln!("[hyte-shell] no 682x2560 panel found, falling back to primary");
            show_fallback(&window)?;
        }
    }

    Ok(())
}

fn main() {
    tauri::Builder::default()
        .setup(setup)
        .run(tauri::generate_context!())
        .expect("error while running hyte-shell");
}
