//! Windows development host with strict GPUI-agent actions installed.

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    nsh_gpui_agent_windows::install();
    nsh_platform_windows::set_ui_sets(vec![nsh_base::registry()]);
    nsh_windows_host::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("nsh-windows-host runs on Windows");
}
