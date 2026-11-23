//! The Native Shell command line with the plug's Android launch hook.

use std::{
    any::Any,
    net::SocketAddr,
    path::PathBuf,
    process::{Command, ExitCode},
};

use anyhow::{Context as _, bail};
use nsh_cli::{AndroidLaunch, set_android_launch_hook};

/// The development host package every app shares in development.
const BUNDLE_ID: &str = "com.longbridge.nsh.host";
/// Where the host reads its token from, in its private files.
const TOKEN_PATH: &str = "files/gpui-agent-token";

/// Runs the command line after installing its hooks.
pub fn main() -> ExitCode {
    let _ = set_android_launch_hook(android_launch);
    nsh_cli::main()
}

/// Hands the agent token to the Android development host when its
/// environment asks for one (`GPUI_AGENT=1` or `true`).
fn android_launch(launch: &mut AndroidLaunch<'_>) -> anyhow::Result<Option<Box<dyn Any>>> {
    if !matches!(std::env::var("GPUI_AGENT").as_deref(), Ok("1" | "true")) {
        return Ok(None);
    }
    let token = std::env::var("GPUI_AGENT_TOKEN")
        .context("GPUI_AGENT_TOKEN is required when GPUI_AGENT=1")?;
    check_agent_token(&token)?;
    let address = std::env::var("GPUI_AGENT_ADDR").unwrap_or_else(|_| "127.0.0.1:17421".into());
    let socket: SocketAddr = address
        .parse()
        .context("GPUI_AGENT_ADDR must be a loopback socket address")?;
    if !socket.ip().is_loopback() {
        bail!("GPUI_AGENT_ADDR must be a loopback socket address");
    }
    let write = shell_command(&[
        "run-as",
        BUNDLE_ID,
        "sh",
        "-c",
        &format!("umask 077 && mkdir -p files && cat > {TOKEN_PATH}"),
    ]);
    launch
        .adb_with_input(&["shell", "-T", &write], &token)
        .context("store the GPUI-agent token in the development host")?;
    launch.intent_extra("GPUI_AGENT", "1");
    launch.intent_extra("GPUI_AGENT_ADDR", &address);
    Ok(Some(Box::new(TokenCleanup {
        adb: launch.adb_path(),
        serial: launch.serial().to_owned(),
    })))
}

/// Removes a token the host did not read (a launch that failed before its
/// activity did), once the run is over.
struct TokenCleanup {
    adb: PathBuf,
    serial: String,
}

impl Drop for TokenCleanup {
    fn drop(&mut self) {
        let remove = shell_command(&["run-as", BUNDLE_ID, "rm", "-f", TOKEN_PATH]);
        let _ = Command::new(&self.adb)
            .args(["-s", &self.serial, "shell", &remove])
            .output();
    }
}

/// A token is compared byte for byte, and whitespace around it does not
/// survive every way to a host, so a token with any is refused.
fn check_agent_token(token: &str) -> anyhow::Result<()> {
    if token.trim().is_empty() {
        bail!("GPUI_AGENT_TOKEN must not be empty");
    }
    if token.trim() != token {
        bail!("GPUI_AGENT_TOKEN must not begin or end with whitespace");
    }
    Ok(())
}

/// `words` as one command of the device's shell, each quoted: `adb shell`
/// joins its arguments with spaces and the device's shell parses them.
fn shell_command(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| {
            let plain = !word.is_empty()
                && word.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, '_' | '-' | '.' | '/' | ':' | ',' | '=' | '@' | '%' | '+')
                });
            if plain {
                (*word).to_owned()
            } else {
                format!("'{}'", word.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{check_agent_token, shell_command};

    /// A token with whitespace around it is refused, not trimmed: hosts
    /// compare it byte for byte.
    #[test]
    fn agent_tokens_with_whitespace_around_them_are_refused() {
        assert!(check_agent_token("0123abcd").is_ok());
        assert!(check_agent_token("01 23").is_ok());
        for token in ["", " ", "\n", " 0123", "0123\n", "\t0123", "0123\r\n"] {
            let error = check_agent_token(token).unwrap_err().to_string();
            assert!(
                error.starts_with("GPUI_AGENT_TOKEN must not"),
                "{token:?}: {error}"
            );
            assert!(
                !error.contains("0123"),
                "the error shows the token: {error}"
            );
        }
    }

    #[test]
    fn shell_commands_quote_what_the_shell_would_read() {
        assert_eq!(
            shell_command(&[
                "run-as",
                "com.longbridge.nsh.host",
                "rm",
                "-f",
                "files/token"
            ]),
            "run-as com.longbridge.nsh.host rm -f files/token"
        );
        assert_eq!(
            shell_command(&["sh", "-c", "cat > files/token"]),
            "sh -c 'cat > files/token'"
        );
        assert_eq!(shell_command(&["it's"]), "'it'\\''s'");
    }
}
