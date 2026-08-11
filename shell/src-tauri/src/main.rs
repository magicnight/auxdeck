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
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOZORDER, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
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
    Some(TargetRect {
        x: r.left,
        y: r.top,
        width: (r.right - r.left).max(0) as u32,
        height: (r.bottom - r.top).max(0) as u32,
    })
}

/// 生产路径：设样式 → 原子定位 → 显示 → 再钉一次（show 跨屏触发的 DPICHANGED
/// 可能让 tao 按 suggested rect 重调，二次 SetWindowPos 把它压回）→ 读回验证。
fn pin_to_panel(window: &WebviewWindow, rect: TargetRect) -> tauri::Result<()> {
    let hwnd = window.hwnd()?;
    apply_noactivate(hwnd);
    set_window_rect(hwnd, rect);
    window.show()?;
    set_window_rect(hwnd, rect);

    match window_rect(hwnd) {
        Some(actual) if actual == rect => {
            eprintln!("[hyte-shell] pinned OK: {rect:?}");
        }
        Some(actual) => {
            eprintln!("[hyte-shell] pin MISMATCH: want {rect:?}, got {actual:?}");
        }
        None => eprintln!("[hyte-shell] pin done but GetWindowRect failed"),
    }
    Ok(())
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
            pin_to_panel(&window, rect)?;
            spawn_position_watchdog(window.clone(), rect);
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
