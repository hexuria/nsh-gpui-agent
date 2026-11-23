//! Plug-owned launcher: installs a custom runtime before invoking stock `nsh run`.

use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

const BUNDLE: &str = "com.longbridge.nsh.host";
const INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>nsh-ios-host</string>
<key>CFBundleIdentifier</key><string>com.longbridge.nsh.host</string>
<key>CFBundleName</key><string>NshDev</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSRequiresIPhoneOS</key><true/>
<key>MinimumOSVersion</key><string>16.0</string>
<key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
</dict></plist>"#;

struct Options {
    runtime: PathBuf,
    device: OsString,
    entry: OsString,
}

fn options(args: impl IntoIterator<Item = OsString>) -> Result<Options, String> {
    let mut args = args.into_iter();
    let mut runtime = None;
    let mut device = None;
    let mut entry = None;
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("every option needs a value")?;
        match arg.to_str() {
            Some("--runtime") => runtime = Some(PathBuf::from(value)),
            Some("--device") => device = Some(value),
            Some("--entry") => entry = Some(value),
            _ => return Err(format!("unknown option: {}", arg.to_string_lossy())),
        }
    }
    Ok(Options {
        runtime: runtime.ok_or("--runtime is required")?,
        device: device.ok_or("--device (a booted Simulator UDID) is required")?,
        entry: entry.ok_or("--entry is required")?,
    })
}

fn output(command: &mut Command) -> Result<String, String> {
    command
        .env_remove("GPUI_AGENT_TOKEN")
        .env_remove("SIMCTL_CHILD_GPUI_AGENT_TOKEN");
    let name = command.get_program().to_string_lossy().into_owned();
    let output = command.output().map_err(|e| format!("{name}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn simctl(args: &[&OsStr]) -> Result<String, String> {
    output(
        Command::new("xcrun")
            .arg("simctl")
            .args(args)
            .env_remove("SIMCTL_CHILD_GPUI_AGENT_TOKEN"),
    )
}

fn check_token(token: &str) -> Result<(), String> {
    if token.is_empty() || token.trim() != token {
        return Err("GPUI_AGENT_TOKEN must be nonempty and have no surrounding whitespace".into());
    }
    Ok(())
}

struct TokenFile(PathBuf);

impl Drop for TokenFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
fn write_token(path: PathBuf, token: &str) -> Result<TokenFile, String> {
    use std::os::unix::fs::OpenOptionsExt;
    check_token(token)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("create private token file: {e}"))?;
    let guard = TokenFile(path);
    file.write_all(token.as_bytes())
        .map_err(|e| format!("write private token file: {e}"))?;
    Ok(guard)
}

#[cfg(not(unix))]
fn write_token(_: PathBuf, _: &str) -> Result<TokenFile, String> {
    Err("the iOS Simulator launcher requires macOS".into())
}

fn run(options: Options) -> Result<ExitCode, String> {
    if !cfg!(target_os = "macos") {
        return Err("the iOS Simulator launcher requires macOS".into());
    }
    if !matches!(std::env::var("GPUI_AGENT").as_deref(), Ok("1" | "true")) {
        return Err("set GPUI_AGENT=1 to opt in".into());
    }
    let token = std::env::var("GPUI_AGENT_TOKEN").map_err(|_| "GPUI_AGENT_TOKEN is required")?;
    check_token(&token)?;
    let address = std::env::var("GPUI_AGENT_ADDR").unwrap_or_else(|_| "127.0.0.1:17421".into());
    if !address
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "invalid GPUI_AGENT_ADDR")?
        .ip()
        .is_loopback()
    {
        return Err("GPUI_AGENT_ADDR must be loopback".into());
    }
    let runtime = options
        .runtime
        .canonicalize()
        .map_err(|e| format!("runtime: {e}"))?;
    let host = runtime.join("ios/iphonesimulator/nsh-ios-host");
    if !host.is_file() {
        return Err(format!("missing host: {}", host.display()));
    }
    let devices: serde_json::Value = serde_json::from_str(&simctl(&[
        OsStr::new("list"),
        OsStr::new("devices"),
        OsStr::new("--json"),
    ])?)
    .map_err(|e| e.to_string())?;
    let booted = devices["devices"]
        .as_object()
        .into_iter()
        .flat_map(|runtimes| runtimes.values())
        .filter_map(|devices| devices.as_array())
        .flatten()
        .any(|device| {
            device["udid"]
                .as_str()
                .is_some_and(|id| OsStr::new(id) == options.device)
                && device["state"] == "Booted"
        });
    if !booted {
        return Err("--device must name a booted Simulator UDID".into());
    }

    // Installing the same bundle ID again preserves its data container. Stock
    // nsh run replaces this bootstrap bundle with its full development bundle.
    let app = runtime.join("NshDev.app");
    fs::create_dir_all(&app).map_err(|e| e.to_string())?;
    fs::copy(host, app.join("nsh-ios-host")).map_err(|e| e.to_string())?;
    fs::write(app.join("Info.plist"), INFO).map_err(|e| e.to_string())?;
    output(
        Command::new("codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(&app),
    )?;
    simctl(&[OsStr::new("install"), &options.device, app.as_os_str()])?;
    let container = simctl(&[
        OsStr::new("get_app_container"),
        &options.device,
        OsStr::new(BUNDLE),
        OsStr::new("data"),
    ])?;
    let path = Path::new(container.trim()).join("Library/Caches/gpui-agent-token");
    let interrupted = Arc::new(AtomicBool::new(false));
    let child_pid = Arc::new(AtomicU32::new(0));
    let flag = interrupted.clone();
    let pid = child_pid.clone();
    let cleanup = path.clone();
    ctrlc::set_handler(move || {
        flag.store(true, Ordering::Release);
        let _ = fs::remove_file(&cleanup);
        interrupt_child(pid.load(Ordering::Acquire));
    })
    .map_err(|e| e.to_string())?;
    let _token = write_token(path, &token)?;
    if interrupted.load(Ordering::Acquire) {
        return Err("interrupted".into());
    }
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .args([
            OsStr::new("run"),
            &options.entry,
            OsStr::new("--target"),
            OsStr::new("ios"),
            OsStr::new("--device"),
            &options.device,
        ])
        .env("NSH_RUNTIME", runtime)
        .env("NSH_IOS_AGENT_CLI_CHILD", "1")
        .env("SIMCTL_CHILD_GPUI_AGENT", "1")
        .env("SIMCTL_CHILD_GPUI_AGENT_ADDR", address)
        .env_remove("SIMCTL_CHILD_GPUI_AGENT_TOKEN")
        .env_remove("GPUI_AGENT_TOKEN");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| format!("nsh run: {e}"))?;
    child_pid.store(child.id(), Ordering::Release);
    if interrupted.load(Ordering::Acquire) {
        interrupt_child(child.id());
    }
    let status = child.wait().map_err(|e| format!("nsh run: {e}"));
    child_pid.store(0, Ordering::Release);
    let status = status?;
    Ok(ExitCode::from(
        status.code().unwrap_or(1).clamp(0, 255) as u8
    ))
}

fn interrupt_child(pid: u32) {
    #[cfg(unix)]
    if pid != 0 {
        // SAFETY: the child is placed in its own process group at spawn;
        // the handler forwards a stop without touching process memory.
        unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGINT) };
    }
    #[cfg(not(unix))]
    let _ = pid;
}

fn main() -> ExitCode {
    if std::env::var_os("NSH_IOS_AGENT_CLI_CHILD").is_some() {
        return nsh_cli::main();
    }
    match options(std::env::args_os().skip(1)).and_then(run) {
        Ok(code) => code,
        Err(error) => {
            eprintln!(
                "{error}\nusage: nsh-gpui-agent-ios --runtime PATH --device UDID --entry PATH"
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_options_and_unknown_options_are_rejected() {
        assert!(options([]).is_err());
        assert!(options(["--unknown", "value"].map(OsString::from)).is_err());
        let options = options(
            [
                "--runtime",
                "runtime",
                "--device",
                "UDID",
                "--entry",
                "gallery",
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(options.entry, "gallery");
    }

    #[test]
    fn invalid_tokens_are_rejected_without_including_the_value() {
        assert!(check_token("valid").is_ok());
        for value in ["", " ", " secret", "secret\n"] {
            assert!(!check_token(value).unwrap_err().contains("secret"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn token_files_are_private_one_use_and_collision_safe() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("ios-launch-token-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("token");
        assert!(write_token(path.clone(), "").is_err());
        assert!(!path.exists());
        let guard = write_token(path.clone(), "first").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_token(path.clone(), "second").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "first");
        drop(guard);
        assert!(!path.exists());
        assert!(write_token(directory.join("missing/token"), "valid").is_err());
        fs::remove_dir(directory).unwrap();
    }
}
