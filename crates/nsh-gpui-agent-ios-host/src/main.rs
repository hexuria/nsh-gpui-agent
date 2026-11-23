//! The standard development host, linked with the iOS surface hook.

#[cfg(target_os = "ios")]
fn main() {
    nsh_gpui_agent::ios::install().expect("install development surface hook");
    nsh_platform_ios::set_ui_sets(vec![nsh_base::registry()]);
    nsh_ios_host::main();
}

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("build this host for aarch64-apple-ios-sim");
    std::process::exit(1);
}
