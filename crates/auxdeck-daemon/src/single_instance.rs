//! 单实例保护：全局命名 mutex `Global\auxdeck-daemon`（CLAUDE.md §3）。
//! 已存在时调用方应记录日志并正常退出（不是 panic）。

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

const MUTEX_NAME: &str = "Global\\auxdeck-daemon";

/// 单实例检测结果。
pub enum SingleInstance {
    /// 本进程持有唯一锁；持有者必须让此值存活到进程退出（OS 会在进程退出时
    /// 自动释放句柄，无需手动 CloseHandle）。
    Acquired(HANDLE),
    /// 已有实例在运行，调用方应记录日志后正常退出。
    AlreadyRunning,
    /// `CreateMutexW` 本身调用失败，无法验证单实例性；按可继续运行处理。
    Unverified,
}

/// 尝试获取单实例锁。
pub fn acquire() -> SingleInstance {
    let wide_name: Vec<u16> = MUTEX_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    match unsafe { CreateMutexW(None, false, PCWSTR(wide_name.as_ptr())) } {
        Ok(handle) => {
            let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
            if already_running {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                SingleInstance::AlreadyRunning
            } else {
                SingleInstance::Acquired(handle)
            }
        }
        Err(err) => {
            tracing::warn!("CreateMutexW failed, continuing without single-instance guard: {err}");
            SingleInstance::Unverified
        }
    }
}
