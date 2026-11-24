use crate::config::Config;
use std::{
    net::{SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::Duration,
};

static CHILD: Mutex<Option<Child>> = Mutex::new(None);

pub fn api_running(config: &Config) -> bool {
    format!("{}:{}", config.api.host, config.api.port)
        .parse::<SocketAddr>()
        .map(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok())
        .unwrap_or(false)
}

pub fn ensure_api(config: &Config) {
    if api_running(config) {
        return;
    }
    let Some(exe) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("api.exe"))) else {
        return;
    };
    if !exe.exists() {
        tracing::warn!("api.exe not found next to Rustify");
        return;
    }
    let mut command = Command::new(&exe);
    command.current_dir(config.dir()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    match command.spawn() {
        Ok(child) => {
            tracing::info!("started Rustify service");
            if let Ok(mut slot) = CHILD.lock() {
                *slot = Some(child);
            }
        }
        Err(e) => tracing::warn!("could not start Rustify service: {e}"),
    }
}

pub fn stop() {
    if let Ok(mut slot) = CHILD.lock()
        && let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
}
