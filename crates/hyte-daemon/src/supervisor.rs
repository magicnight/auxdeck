//! 守护 hyte-shell.exe：在 daemon 同目录查找，存在则拉起并监控，异常退出时
//! 指数退避重启；不存在则 warn 一次跳过——开发模式常态（CLAUDE.md 任务书 5）。

use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::Instant;
use tracing::{error, info, warn};

const SHELL_EXE_NAME: &str = "hyte-shell.exe";
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const STABLE_RUN_THRESHOLD: Duration = Duration::from_secs(60);

fn shell_exe_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join(SHELL_EXE_NAME);
    candidate.exists().then_some(candidate)
}

/// 持续监控 hyte-shell.exe。若同目录下不存在该可执行文件，记录一次 warn 后
/// 直接返回，不重试（开发阶段 shell 尚未编译是常态）。
pub async fn run() {
    let Some(shell_path) = shell_exe_path() else {
        warn!(
            "{SHELL_EXE_NAME} not found next to daemon executable, skipping shell supervision \
             (expected in dev mode)"
        );
        return;
    };

    let mut backoff = INITIAL_BACKOFF;

    loop {
        info!("starting {}", shell_path.display());
        let started_at = Instant::now();

        match Command::new(&shell_path).spawn() {
            Ok(mut child) => match child.wait().await {
                Ok(status) => info!("hyte-shell exited with {status}"),
                Err(err) => error!("failed to wait on hyte-shell process: {err}"),
            },
            Err(err) => error!("failed to spawn {}: {err}", shell_path.display()),
        }

        if started_at.elapsed() >= STABLE_RUN_THRESHOLD {
            backoff = INITIAL_BACKOFF;
        }

        warn!("restarting hyte-shell in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}
