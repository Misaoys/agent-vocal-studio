#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    #[cfg(windows)]
    if let Err(error) = agent_vocal_studio::run_desktop() {
        let message = format!("无法启动音乐编辑器\n{error:#}\n");
        eprintln!("{message}");
        if let Ok(exe) = std::env::current_exe() {
            if let Some(home) = exe.parent().and_then(std::path::Path::parent) {
                let runtime = home.join("runtime");
                let _ = std::fs::create_dir_all(&runtime);
                let _ = std::fs::write(runtime.join("desktop-startup-error.txt"), message);
            }
        }
        std::process::exit(1);
    }
}
