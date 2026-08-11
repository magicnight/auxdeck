//! tracing 初始化：默认只写 `%APPDATA%\auxdeck\logs\` 按天滚动文件，不写
//! stdout；`--console` 时额外输出到 stdout（CLAUDE.md §10）。

use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// 初始化全局 tracing subscriber。返回的 guard 必须保留到进程退出——一旦
/// 提前 drop，后台写线程会被关闭，文件日志可能丢失最后几行。
pub fn init(console: bool) -> WorkerGuard {
    let log_dir = log_dir();
    if let Err(err) = std::fs::create_dir_all(&log_dir) {
        eprintln!("failed to create log dir {}: {err}", log_dir.display());
    }

    let file_appender = tracing_appender::rolling::daily(&log_dir, "auxdeck-daemon.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let file_layer = fmt::layer().with_writer(non_blocking).with_ansi(false);
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer);

    if console {
        registry
            .with(fmt::layer().with_writer(std::io::stdout))
            .init();
    } else {
        registry.init();
    }

    guard
}

fn log_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("auxdeck").join("logs")
}
