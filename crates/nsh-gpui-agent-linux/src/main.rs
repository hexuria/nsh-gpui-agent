//! The Linux development host with strict native agent actions.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    nsh_gpui_agent_linux::install();
    nsh_platform_linux::set_ui_sets(vec![nsh_base::registry()]);
    nsh_linux_host::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("nsh-linux-host runs on Linux");
    std::process::ExitCode::FAILURE
}
