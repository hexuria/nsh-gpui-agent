//! Opt-in GPUI-agent Web sessions for a binary built on the NSH CLI.

mod cdp;
mod pairing;
mod synth;
mod tab_native;
mod ui;
mod web_agent;
mod web_native;

/// Runs the NSH CLI with opt-in Web automation support.
pub fn main() -> std::process::ExitCode {
    if let Err(error) = web_agent::config() {
        eprintln!("error: {error:#}");
        return std::process::ExitCode::FAILURE;
    }
    if nsh_cli::set_web_session_hook(attach).is_err() {
        eprintln!("error: a web session hook is already installed");
        return std::process::ExitCode::FAILURE;
    }
    nsh_cli::main()
}

fn attach(session: &nsh_cli::WebSession<'_>) -> anyhow::Result<Option<Box<dyn std::any::Any>>> {
    let Some((addr, token)) = web_agent::config()? else {
        return Ok(None);
    };
    anyhow::ensure!(
        !session.rust_root,
        "GPUI_AGENT=1 is not available for a Rust web root"
    );
    Ok(Some(Box::new(web_agent::start(
        session.server.clone(),
        session.name,
        addr,
        token,
        web_agent::tab_mode(),
    )?)))
}
