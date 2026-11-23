pub(crate) use nsh_cli::{Stoppable, note, stop_requested};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::Mutex,
};

pub(crate) fn path(path: &std::path::Path) -> String {
    path.display().to_string()
}

#[derive(Clone, Default)]
pub(crate) struct Tail(std::sync::Arc<Mutex<std::collections::VecDeque<String>>>);

impl Tail {
    /// The kept lines.
    pub(crate) fn text(&self) -> String {
        let lines = self.0.lock().unwrap_or_else(|e| e.into_inner());
        lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Spawns `cmd`, a process that runs alongside the CLI (a host): its own
/// output is kept, not shown ([`Tail`]); verbose runs show it.
pub(crate) fn quiet(cmd: &mut Command) -> std::io::Result<(std::process::Child, Tail)> {
    const KEEP: usize = 200;
    let tail = Tail::default();
    let (reader, writer) = std::io::pipe()?;
    cmd.stdout(writer.try_clone()?).stderr(writer);
    let child = cmd.spawn();
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let child = child?;
    let lines = tail.clone();
    std::thread::Builder::new()
        .name("nsh-host-output".into())
        .spawn(move || {
            for line in BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                if std::env::args_os().any(|arg| arg == "--verbose" || arg == "-v")
                    || std::env::var("NSH_VERBOSE")
                        .is_ok_and(|v| matches!(v.as_str(), "true" | "1" | "yes" | "on"))
                {
                    eprintln!("{line}");
                }
                let mut lines = lines.0.lock().unwrap_or_else(|e| e.into_inner());
                if lines.len() == KEEP {
                    lines.pop_front();
                }
                lines.push_back(line);
            }
        })?;
    Ok((child, tail))
}
