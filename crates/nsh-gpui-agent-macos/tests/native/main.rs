//! Native agent checks need AppKit on the process main thread and a desktop session.
//!
//! Run the strict native checks with:
//! `GPUI_AGENT=0 NSH_NATIVE_AGENT_CHECK_ONLY=1 cargo test -p nsh-gpui-agent-macos --features dev-agent --test native -- --ignored`
//! Run the shared adapter listener checks with:
//! `NSH_AGENT_LISTENER_CHECK_ONLY=1 cargo test -p nsh-gpui-agent-macos --features dev-agent --test native -- --ignored`

#[cfg(all(target_os = "macos", not(feature = "gpui-agent-server")))]
mod agent_listener;
#[cfg(target_os = "macos")]
mod agent_support;
#[cfg(target_os = "macos")]
mod native_agent;

fn main() {
    // Native checks drive AppViews in-process; a development shell's
    // GPUI_AGENT settings must not open the shared agent listener here.
    // SAFETY: no other thread exists yet to read the environment.
    unsafe { std::env::remove_var("GPUI_AGENT") };
    if !std::env::args().any(|arg| arg == "--ignored") {
        println!("Native AppKit agent checks skipped; run with --ignored in a desktop session.");
        return;
    }
    #[cfg(target_os = "macos")]
    {
        nsh_gpui_agent_macos::install();
        if std::env::var_os("NSH_AGENT_LISTENER_CHECK_ONLY").is_some() {
            #[cfg(not(feature = "gpui-agent-server"))]
            agent_listener::run();
            #[cfg(feature = "gpui-agent-server")]
            panic!("the adapter listener checks must run without gpui-agent-server");
        } else {
            native_agent::run();
        }
    }
}
