//! The macOS development host with strict native agent actions.

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    nsh_gpui_agent_macos::install();
    nsh_platform_macos::set_ui_sets(vec![nsh_base::registry()]);
    nsh_macos_host::main()
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!("nsh-macos-host runs on macOS");
    std::process::ExitCode::FAILURE
}
