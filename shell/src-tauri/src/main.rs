//! hyte-shell：Tauri v2 渲染层（CLAUDE.md §3 / §9）。
//! M1：窗口钉在目标副屏、附加 NOACTIVATE 不抢焦点、每 5s 巡检位置漂移并搬回。
//! 找不到目标屏时退化为开发态：主屏、带边框、不设 NOACTIVATE（CLAUDE.md §9.1）。

use std::time::Duration;

use tauri::{App, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// 目标副屏物理像素尺寸（CLAUDE.md §2）。显示方向可能纵向或纵向翻转，宽高互换也算命中。
const PANEL_SIZE_A: (u32, u32) = (682, 2560);
const PANEL_SIZE_B: (u32, u32) = (2560, 682);

/// 找不到目标屏时的开发态兜底窗口尺寸。
const FALLBACK_WIDTH: u32 = 400;
const FALLBACK_HEIGHT: u32 = 800;

/// 位置巡检间隔，对应 M1 验收标准 3「5s 内回到副屏正确位置」。
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);

/// 目标副屏在桌面坐标系中的矩形，setup 阶段发现一次，巡检线程据此纠偏。
/// WM_DISPLAYCHANGE 子类化、按 EDID 重新枚举留待后续里程碑，M1 不做。
#[derive(Clone, Copy)]
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

/// 生产路径：铺满目标屏、附加 NOACTIVATE、再显示（先隐藏后定位是为了避免闪一下主屏）。
fn pin_to_panel(window: &WebviewWindow, rect: TargetRect) -> tauri::Result<()> {
    window.set_position(Position::Physical(PhysicalPosition {
        x: rect.x,
        y: rect.y,
    }))?;
    window.set_size(Size::Physical(PhysicalSize {
        width: rect.width,
        height: rect.height,
    }))?;

    let hwnd = window.hwnd()?;
    apply_noactivate(hwnd);

    window.show()?;
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

/// 每 5s 校验窗口是否仍铺满目标屏 rect，漂移（待机唤醒 / 独占全屏切换踢回主屏、
/// 分辨率变化被系统改尺寸等）则位置与尺寸一并复位。
fn spawn_position_watchdog(window: WebviewWindow, rect: TargetRect) {
    std::thread::spawn(move || loop {
        std::thread::sleep(WATCHDOG_INTERVAL);
        let Ok(pos) = window.outer_position() else {
            continue;
        };
        let Ok(size) = window.outer_size() else {
            continue;
        };
        let drifted = pos.x != rect.x
            || pos.y != rect.y
            || size.width != rect.width
            || size.height != rect.height;
        if drifted {
            let _ = window.set_position(Position::Physical(PhysicalPosition {
                x: rect.x,
                y: rect.y,
            }));
            let _ = window.set_size(Size::Physical(PhysicalSize {
                width: rect.width,
                height: rect.height,
            }));
        }
    });
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app
        .get_webview_window("main")
        .expect("tauri.conf.json 声明的 main 窗口必须存在");

    let target = app
        .available_monitors()?
        .into_iter()
        .find(|monitor| is_panel_monitor(monitor.size()));

    match target {
        Some(monitor) => {
            let rect = TargetRect {
                x: monitor.position().x,
                y: monitor.position().y,
                width: monitor.size().width,
                height: monitor.size().height,
            };
            pin_to_panel(&window, rect)?;
            spawn_position_watchdog(window.clone(), rect);
        }
        None => {
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
