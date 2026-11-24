//! A Chromium browser (Chrome, Edge, Chromium) driven through the Chrome
//! DevTools Protocol: what `nsh test --target web` runs apps in. The
//! browser runs headless on a profile of its own; each app gets a page in a
//! browser context of its own (storage and cookies apart, as a fresh
//! profile's), and its input is the browser's trusted input
//! (`Input.dispatch*`), which pages cannot tell from a user's.
//!
//! nsh finds the browser installed on this machine ([`find`]) and never
//! fetches one. The protocol's calls go one at a time, each waiting for its
//! answer; the few events the browser sends without being asked are
//! skipped.
//!
//! On Linux and macOS the protocol goes over two pipes the browser inherits
//! (`--remote-debugging-pipe`): it reads calls on its descriptor 3 and
//! writes answers and events on its descriptor 4, each message a JSON
//! document ended by a NUL byte. The browser listens on no socket, so no
//! other process on the machine can drive it. No other descriptor of this
//! process reaches the browser.
//!
//! On Windows the protocol still goes over a WebSocket on a loopback port
//! the browser picks (`--remote-debugging-port=0`), which any process on
//! the machine can connect to while the browser runs. Chromium's pipes
//! there (`--remote-debugging-io-pipes`) need handles the browser inherits,
//! and the standard library cannot limit a child to the handles it is
//! meant to have: the pipes would reach every process this one starts
//! meanwhile.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::ui;

/// How long the browser may take to start.
const START: Duration = Duration::from_secs(30);
/// How long a call may take.
const CALL: Duration = Duration::from_secs(30);
/// How often a read looks at the clock while nothing comes.
const POLL: Duration = Duration::from_millis(50);

/// The browser to run apps in: `given` (`--browser`), else Chrome, Edge or
/// Chromium where they install on this system; an error saying how to get
/// one when there is none.
pub(crate) fn find(given: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(path) = given {
        if path.is_file() {
            return Ok(path.to_owned());
        }
        anyhow::bail!("--browser {}: no such file", ui::path(path));
    }
    let candidates = candidates();
    if let Some(found) = candidates.iter().find(|p| p.is_file()) {
        return Ok(found.clone());
    }
    let looked = if cfg!(any(target_os = "macos", windows)) {
        candidates
            .iter()
            .map(|p| format!("\n  {}", p.display()))
            .collect::<String>()
    } else {
        format!(" {} on PATH", LINUX_NAMES.join(", "))
    };
    anyhow::bail!(
        "nsh test --target web runs the apps in Chrome, Edge or Chromium, and none is installed where nsh looks:{looked}\n\
         install one, or name its executable with --browser <PATH>"
    )
}

/// The names Chrome, Edge and Chromium have on Linux and the BSDs.
const LINUX_NAMES: [&str; 7] = [
    "google-chrome",
    "google-chrome-stable",
    "microsoft-edge",
    "microsoft-edge-stable",
    "chromium",
    "chromium-browser",
    "chrome",
];

/// Where Chrome, Edge and Chromium install, in that order.
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if cfg!(target_os = "macos") {
        let apps = [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Chromium.app/Contents/MacOS/Chromium",
        ];
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        for app in apps {
            paths.extend(roots.iter().map(|root| root.join(app)));
        }
    } else if cfg!(windows) {
        let apps = [
            r"Google\Chrome\Application\chrome.exe",
            r"Microsoft\Edge\Application\msedge.exe",
            r"Chromium\Application\chrome.exe",
        ];
        let roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect();
        for app in apps {
            paths.extend(roots.iter().map(|root| root.join(app)));
        }
    } else {
        let dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        for name in LINUX_NAMES {
            paths.extend(dirs.iter().map(|dir| dir.join(name)));
        }
    }
    paths
}

/// A headless browser this process started; it goes when it is closed
/// ([`Browser::close`]) or dropped.
pub(crate) struct Browser {
    child: Mutex<Child>,
    transport: Mutex<transport::Transport>,
    next: AtomicU64,
    /// Its profile, removed after it.
    profile: Profile,
    output: ui::Tail,
    closed: AtomicBool,
}

impl Browser {
    /// Starts `exe` headless on a fresh profile and connects to it.
    #[allow(dead_code)]
    pub(crate) fn launch(exe: &Path) -> Result<Browser, String> {
        Browser::launch_with(exe, false)
    }

    /// Starts `exe` on a fresh profile, `headed` in a window of its own
    /// (to watch what an agent does), else headless, and connects to it.
    pub(crate) fn launch_with(exe: &Path, headed: bool) -> Result<Browser, String> {
        let profile = std::env::temp_dir().join(format!(
            "nsh-test-browser-{}-{}",
            std::process::id(),
            NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
        ));
        Self::launch_in(exe, headed, profile)
    }

    fn launch_in(exe: &Path, headed: bool, profile: PathBuf) -> Result<Browser, String> {
        let profile = Profile(profile);
        let _ = std::fs::remove_dir_all(&profile.0);
        std::fs::create_dir_all(&profile.0)
            .map_err(|e| format!("cannot make {}: {e}", profile.0.display()))?;
        let mut command = command(exe, headed, &profile.0);
        let (child, output, transport) = transport::start(&mut command, exe, &profile.0)?;
        Ok(Browser {
            child: Mutex::new(child),
            transport: Mutex::new(transport),
            next: AtomicU64::new(1),
            profile,
            output,
            closed: AtomicBool::new(false),
        })
    }

    /// Whether it still runs.
    pub(crate) fn alive(&self) -> bool {
        matches!(self.lock_child().try_wait(), Ok(None))
    }

    /// What it printed last, on lines of their own (for errors).
    pub(crate) fn output(&self) -> String {
        indented(&self.output.text())
    }

    /// A blank page in a browser context of its own.
    pub(crate) fn page(self: &Arc<Self>) -> Result<Page, String> {
        let id = |value: Value, key: &str| {
            value[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("the browser answered without a {key}"))
        };
        let context = id(
            self.call(None, "Target.createBrowserContext", json!({}))?,
            "browserContextId",
        )?;
        // Dropped on the way out, it removes the context again.
        let mut page = Page {
            browser: self.clone(),
            context,
            target: String::new(),
            session: String::new(),
            closed: AtomicBool::new(false),
        };
        page.target = id(
            self.call(
                None,
                "Target.createTarget",
                json!({ "url": "about:blank", "browserContextId": page.context }),
            )?,
            "targetId",
        )?;
        page.session = id(
            self.call(
                None,
                "Target.attachToTarget",
                json!({ "targetId": page.target, "flatten": true }),
            )?,
            "sessionId",
        )?;
        Ok(page)
    }

    /// Calls `method` with `params`, on the page of `session` or the
    /// browser: its result, or the browser's error.
    fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = session.into();
        }
        let mut transport = self.transport.lock().unwrap_or_else(|e| e.into_inner());
        transport
            .send(message.to_string())
            .map_err(|e| format!("the browser is gone: {e}"))?;
        let deadline = Instant::now() + CALL;
        loop {
            match transport.receive() {
                Received::Message(text) => {
                    let Ok(reply) = serde_json::from_slice::<Value>(&text) else {
                        continue;
                    };
                    // Events, and answers to calls that gave up waiting.
                    if reply.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(error) = reply.get("error") {
                        let text = error["message"].as_str().unwrap_or("an error");
                        return Err(format!("the browser refused {method}: {text}"));
                    }
                    return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
                }
                Received::Nothing => {
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "the browser did not answer {method} in {} s",
                            CALL.as_secs()
                        ));
                    }
                }
                Received::Gone(e) => return Err(format!("the browser is gone: {e}")),
            }
        }
    }

    fn lock_child(&self) -> std::sync::MutexGuard<'_, Child> {
        self.child.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Closes it and removes its profile, once; the pages left fail their
    /// calls.
    pub(crate) fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // Closed as a user closes it, so its helper processes go with it.
        let _ = self.call(None, "Browser.close", json!({}));
        let mut child = self.lock_child();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        drop(child);
        let _ = std::fs::remove_dir_all(&self.profile.0);
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.close();
    }
}

struct Profile(PathBuf);

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The command that starts `exe` on `profile`, `headed` in a window of its
/// own, else headless.
fn command(exe: &Path, headed: bool, profile: &Path) -> Command {
    let mut command = Command::new(exe);
    if !headed {
        command.arg("--headless");
    }
    command
        .arg(transport::FLAG)
        .arg(format!("--user-data-dir={}", profile.display()))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            "--mute-audio",
            // Pages in the background keep their timers and frames: a
            // test's page is never in front.
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            // Headless, the browser finds no input device and reports no
            // pointer (`pointer: none`, `hover: none`), so pages measure
            // as touch screens; the test drives a mouse: a fine pointer
            // that hovers (Blink's `PointerType` / `HoverType`, as
            // Playwright launches it).
            "--blink-settings=primaryPointerType=4,availablePointerTypes=4,\
             primaryHoverType=2,availableHoverTypes=2",
            // No downloads of its own (components, Safe Browsing lists):
            // they made a profile in the temp directory 150 MB.
            "--disable-component-update",
            "--disable-background-networking",
            "about:blank",
        ])
        .stdin(Stdio::null())
        // A GPUI agent's token (`nsh run`'s environment) is the agent
        // server's alone: the browser and its pages never hold it.
        .env_remove("GPUI_AGENT_TOKEN");
    command
}

/// The profiles started so far: each browser gets one of its own.
static NEXT_PROFILE: AtomicU64 = AtomicU64::new(0);

/// A page of the browser, in a browser context of its own: closing it
/// (or dropping it) removes the context and what the page kept.
pub(crate) struct Page {
    browser: Arc<Browser>,
    context: String,
    /// Its target, closed before the context goes.
    target: String,
    session: String,
    closed: AtomicBool,
}

impl Page {
    /// Calls `method` with `params` on this page.
    pub(crate) fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.browser.call(Some(&self.session), method, params)
    }

    /// Whether it was closed ([`Page::close`]).
    pub(crate) fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// The browser it is in.
    pub(crate) fn browser(&self) -> &Browser {
        &self.browser
    }

    /// Closes the page and its context. The page goes first, with
    /// `Target.closeTarget`, which runs none of its `beforeunload` listeners:
    /// an app listening to quit requests is not asked, and no leave dialog
    /// holds the page.
    pub(crate) fn close(&self) {
        if !self.closed.swap(true, Ordering::Relaxed) {
            if !self.target.is_empty() {
                let _ = self.browser.call(
                    None,
                    "Target.closeTarget",
                    json!({ "targetId": self.target }),
                );
            }
            let _ = self.browser.call(
                None,
                "Target.disposeBrowserContext",
                json!({ "browserContextId": self.context }),
            );
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        self.close();
    }
}

/// What a read of the browser's messages gave.
enum Received {
    /// An answer or an event.
    Message(Vec<u8>),
    /// Nothing, for [`POLL`].
    Nothing,
    /// The browser is gone, and why.
    Gone(String),
}

/// `exe`, started as `child`, did not start: it is stopped, and the error
/// says how it went (`exited`: it had gone by itself).
fn not_started(child: &mut Child, exe: &Path, output: &ui::Tail, why: String) -> String {
    if let Ok(Some(status)) = child.try_wait() {
        return format!(
            "{} exited with {status} as it started{}",
            exe.display(),
            indented(&output.text())
        );
    }
    let _ = child.kill();
    let _ = child.wait();
    format!("{why}{}", indented(&output.text()))
}

/// The protocol over two pipes the browser inherits as its descriptors 3
/// (calls) and 4 (answers and events).
#[cfg(unix)]
mod transport {
    use std::{
        io::{self, BufRead, BufReader, PipeReader, PipeWriter, Write},
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
            unix::process::CommandExt,
        },
        path::Path,
        process::{Child, Command},
        sync::mpsc,
        time::{Duration, Instant},
    };

    use serde_json::{Value, json};

    use super::{POLL, Received, START, not_started};
    use crate::ui;

    /// The flag that hands the browser the pipes; it opens no port.
    pub(super) const FLAG: &str = "--remote-debugging-pipe";

    /// The lowest descriptor the browser's ends have here, above the 3 and
    /// 4 the browser gets them as: putting one in place never closes the
    /// other. Every descriptor from here up is closed in the browser.
    const ABOVE: RawFd = 5;

    pub(super) struct Transport {
        calls: PipeWriter,
        /// The browser's messages, read by a thread of their own.
        messages: mpsc::Receiver<Vec<u8>>,
    }

    impl Transport {
        pub(super) fn send(&mut self, text: String) -> io::Result<()> {
            let mut bytes = text.into_bytes();
            bytes.push(0);
            self.calls.write_all(&bytes)
        }

        pub(super) fn receive(&mut self) -> Received {
            match self.messages.recv_timeout(POLL) {
                Ok(message) => Received::Message(message),
                Err(mpsc::RecvTimeoutError::Timeout) => Received::Nothing,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    Received::Gone("its DevTools pipe closed".into())
                }
            }
        }
    }

    /// Starts `command` (`exe`) with the pipes, and waits for its first
    /// answer: the browser, its output and the pipes.
    pub(super) fn start(
        command: &mut Command,
        exe: &Path,
        _profile: &Path,
    ) -> Result<(Child, ui::Tail, Transport), String> {
        let cannot = |e: io::Error| format!("cannot start {}: {e}", exe.display());
        let (browser_calls, calls) = io::pipe().map_err(cannot)?;
        let (messages, browser_messages) = io::pipe().map_err(cannot)?;
        let browser_calls = above(browser_calls.into()).map_err(cannot)?;
        let browser_messages = above(browser_messages.into()).map_err(cannot)?;
        let messages = read(messages).map_err(cannot)?;
        inherit(
            command,
            browser_calls.as_raw_fd(),
            browser_messages.as_raw_fd(),
        );
        let spawned = ui::quiet(command);
        // The browser's ends are its alone: once it is gone, reads here end.
        drop((browser_calls, browser_messages));
        let (mut child, output) = spawned.map_err(cannot)?;
        let mut transport = Transport { calls, messages };
        let deadline = Instant::now() + START;
        let hello = json!({ "id": 0, "method": "Browser.getVersion" }).to_string();
        if let Err(e) = transport.send(hello) {
            let why = format!("cannot connect to {}: {e}", exe.display());
            return Err(not_started(&mut child, exe, &output, why));
        }
        loop {
            match transport.receive() {
                Received::Message(text) => {
                    let id = serde_json::from_slice::<Value>(&text)
                        .ok()
                        .and_then(|reply| reply.get("id").and_then(Value::as_u64));
                    if id == Some(0) {
                        return Ok((child, output, transport));
                    }
                }
                Received::Nothing => {
                    if matches!(child.try_wait(), Ok(Some(_))) || Instant::now() >= deadline {
                        let why =
                            format!("{} did not start in {} s", exe.display(), START.as_secs());
                        return Err(not_started(&mut child, exe, &output, why));
                    }
                }
                Received::Gone(e) => {
                    // A browser closes its pipes as it exits: a moment for
                    // it to have exited.
                    let exits = Instant::now() + Duration::from_secs(1);
                    while matches!(child.try_wait(), Ok(None)) && Instant::now() < exits {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let why = format!("cannot connect to {}: {e}", exe.display());
                    return Err(not_started(&mut child, exe, &output, why));
                }
            }
        }
    }

    /// `fd` again, at [`ABOVE`] or higher, closed on exec.
    fn above(fd: OwnedFd) -> io::Result<OwnedFd> {
        // SAFETY: `fd` is open; the copy is owned by nothing else.
        let copy = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, ABOVE) };
        if copy < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `copy` was just opened, and is owned here alone.
        Ok(unsafe { OwnedFd::from_raw_fd(copy) })
    }

    /// Makes the browser's ends its descriptors 3 and 4, and closes every
    /// descriptor from [`ABOVE`] up when it starts (whatever else of this
    /// process was not to be closed on exec).
    fn inherit(command: &mut Command, calls: RawFd, messages: RawFd) {
        let limit = open_limit();
        // SAFETY: between fork and exec the closure only calls `dup2` and
        // `fcntl`, which are async-signal-safe, and allocates nothing.
        // Descriptors are marked closed-on-exec, not closed: the standard
        // library reports a failed exec through one of them.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(calls, 3) < 0 || libc::dup2(messages, 4) < 0 {
                    return Err(io::Error::last_os_error());
                }
                for fd in ABOVE..limit {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }
    }

    /// One more than the highest descriptor this process can open (its
    /// `RLIMIT_NOFILE`, at most 2^20).
    fn open_limit() -> RawFd {
        const MOST: RawFd = 1 << 20;
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `getrlimit` only writes `limit`.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
            return MOST;
        }
        RawFd::try_from(limit.rlim_cur).map_or(MOST, |n| n.min(MOST))
    }

    /// The messages on `pipe`, as a thread reads them; it ends when the
    /// browser closes its end.
    fn read(pipe: PipeReader) -> io::Result<mpsc::Receiver<Vec<u8>>> {
        let (sender, messages) = mpsc::channel();
        std::thread::Builder::new()
            .name("nsh-browser-pipe".into())
            .spawn(move || {
                let mut pipe = BufReader::new(pipe);
                loop {
                    let mut message = Vec::new();
                    // A message without its NUL: the browser went as it
                    // wrote.
                    match pipe.read_until(0, &mut message) {
                        Ok(_) if message.pop() == Some(0) => {
                            if sender.send(message).is_err() {
                                return;
                            }
                        }
                        _ => return,
                    }
                }
            })?;
        Ok(messages)
    }
}

/// The protocol over a WebSocket on a loopback port the browser picks: see
/// the module's documentation for why Windows does not use the pipes.
#[cfg(windows)]
mod transport {
    use std::{
        net::TcpStream,
        path::Path,
        process::{Child, Command},
        time::{Duration, Instant},
    };

    use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

    use super::{POLL, Received, START, not_started};
    use crate::ui;

    /// The port it picks goes into the profile's `DevToolsActivePort`.
    pub(super) const FLAG: &str = "--remote-debugging-port=0";

    pub(super) struct Transport(WebSocket<MaybeTlsStream<TcpStream>>);

    impl Transport {
        pub(super) fn send(&mut self, text: String) -> Result<(), tungstenite::Error> {
            self.0.send(Message::text(text))
        }

        pub(super) fn receive(&mut self) -> Received {
            match self.0.read() {
                Ok(Message::Text(text)) => Received::Message(text.as_str().as_bytes().to_vec()),
                Ok(_) => Received::Nothing,
                Err(tungstenite::Error::Io(err))
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    Received::Nothing
                }
                Err(e) => Received::Gone(e.to_string()),
            }
        }
    }

    /// Starts `command` (`exe`) on `profile`, waits for the port it picks
    /// and connects to it: the browser, its output and the connection.
    pub(super) fn start(
        command: &mut Command,
        exe: &Path,
        profile: &Path,
    ) -> Result<(Child, ui::Tail, Transport), String> {
        let (mut child, output) =
            ui::quiet(command).map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
        let port_file = profile.join("DevToolsActivePort");
        let deadline = Instant::now() + START;
        let url = loop {
            if let Ok(text) = std::fs::read_to_string(&port_file) {
                let mut lines = text.lines();
                if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                    break format!("ws://127.0.0.1:{}{}", port.trim(), path.trim());
                }
            }
            if matches!(child.try_wait(), Ok(Some(_))) || Instant::now() >= deadline {
                let why = format!("{} did not start in {} s", exe.display(), START.as_secs());
                return Err(not_started(&mut child, exe, &output, why));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let socket = match tungstenite::connect(url.as_str()) {
            Ok((socket, _)) => socket,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot connect to {}: {e}", exe.display()));
            }
        };
        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            let _ = stream.set_read_timeout(Some(POLL));
            let _ = stream.set_nodelay(true);
        }
        Ok((child, output, Transport(socket)))
    }
}

/// `text` (the browser's output) on lines of its own, if there is any.
fn indented(text: &str) -> String {
    if text.trim().is_empty() {
        String::new()
    } else {
        format!(":\n{}", text.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_spawn_removes_the_profile() {
        let profile = std::env::temp_dir().join(format!("nsh-failed-spawn-{}", std::process::id()));
        assert!(Browser::launch_in(Path::new("/no/such/browser"), false, profile.clone()).is_err());
        assert!(!profile.exists());
    }

    /// A fake browser, `script` (with its `#!` line), in a directory of its
    /// own: the directory, the browser and the profile to start it on.
    #[cfg(unix)]
    fn fake_browser(name: &str, script: &str) -> (PathBuf, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("nsh-fake-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let exe = root.join("browser");
        std::fs::write(&exe, script).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let profile = root.join("profile");
        (root, exe, profile)
    }

    #[cfg(unix)]
    fn failed_launch_fixture(name: &str, body: &str) -> (String, bool) {
        let (root, exe, profile) =
            fake_browser(&format!("failed-{name}"), &format!("#!/bin/sh\n{body}"));
        let error = Browser::launch_in(&exe, false, profile.clone())
            .err()
            .unwrap();
        let profile_removed = !profile.exists();
        std::fs::remove_dir_all(root).unwrap();
        (error, profile_removed)
    }

    #[test]
    #[cfg(unix)]
    fn early_exit_removes_the_profile() {
        let (error, profile_removed) = failed_launch_fixture("exit", "exit 1\n");
        assert!(error.contains("exited"), "{error}");
        assert!(profile_removed);
    }

    #[test]
    #[cfg(unix)]
    fn failed_connection_removes_the_profile() {
        // It runs on, but closes the pipes without answering.
        let (error, profile_removed) =
            failed_launch_fixture("connect", "exec 3<&- 4>&-\nexec sleep 60\n");
        assert!(error.contains("cannot connect"), "{error}");
        assert!(profile_removed);
    }

    /// A browser that answers each call on its descriptor 4, after an event,
    /// with whether descriptor `leak` reached it, and exits once closed.
    #[cfg(unix)]
    fn pipe_browser(leak: i32) -> String {
        format!(
            r#"#!/usr/bin/env bash
leaked=false
[ -e /dev/fd/{leak} ] && leaked=true
while IFS= read -r -d '' call <&3; do
  id=${{call#*\"id\":}}
  id=${{id%%,*}}
  case "$call" in
    *'"Browser.close"'*) printf '{{"id":%s,"result":{{}}}}\0' "$id" >&4; exit 0;;
    *'"Fake.refuse"'*) printf '{{"id":%s,"error":{{"message":"no"}}}}\0' "$id" >&4;;
    *) printf '{{"method":"Fake.event","params":{{"id":%s}}}}\0{{"id":%s,"result":{{"leaked":%s}}}}\0' "$id" "$id" "$leaked" >&4;;
  esac
done
"#
        )
    }

    #[test]
    #[cfg(unix)]
    fn the_browser_talks_over_its_pipes_and_inherits_nothing_else() {
        // A descriptor of this process not closed on exec, above 3 and 4.
        // SAFETY: `fcntl` copies standard error; the copy is closed below.
        let leak = unsafe { libc::fcntl(2, libc::F_DUPFD, 10) };
        assert!(leak >= 10);
        let (root, exe, profile) = fake_browser("pipe", &pipe_browser(leak));
        let browser = Browser::launch_in(&exe, false, profile.clone()).unwrap();
        // SAFETY: `leak` is ours and open.
        unsafe { libc::close(leak) };
        assert_eq!(
            browser.call(None, "Fake.call", json!({})).unwrap(),
            json!({ "leaked": false })
        );
        assert_eq!(
            browser
                .call(Some("session"), "Fake.refuse", json!({}))
                .unwrap_err(),
            "the browser refused Fake.refuse: no"
        );
        assert!(browser.alive());
        browser.close();
        assert!(!browser.alive());
        assert!(!profile.exists());
        assert!(
            browser
                .call(None, "Fake.call", json!({}))
                .unwrap_err()
                .contains("gone")
        );
        drop(browser);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn the_browser_opens_no_devtools_port() {
        for headed in [false, true] {
            let command = command(Path::new("chrome"), headed, Path::new("/tmp/profile"));
            let args: Vec<_> = command.get_args().collect();
            assert!(args.iter().any(|arg| *arg == "--remote-debugging-pipe"));
            assert!(
                !args
                    .iter()
                    .any(|arg| arg.to_string_lossy().starts_with("--remote-debugging-port")),
                "{args:?}"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn on_windows_the_browser_still_opens_a_loopback_port() {
        let command = command(Path::new("chrome"), false, Path::new("C:\\profile"));
        assert!(
            command
                .get_args()
                .any(|arg| arg == "--remote-debugging-port=0")
        );
        assert!(
            !command
                .get_args()
                .any(|arg| arg == "--remote-debugging-pipe")
        );
    }

    #[test]
    #[cfg(not(any(target_os = "macos", windows)))]
    fn stock_chrome_is_the_last_path_candidate() {
        assert_eq!(LINUX_NAMES.last(), Some(&"chrome"));
        let dirs: Vec<_> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
        let paths = candidates();
        let chrome: Vec<_> = dirs.iter().map(|dir| dir.join("chrome")).collect();
        assert!(paths.ends_with(&chrome));
        assert_eq!(paths.len(), dirs.len() * LINUX_NAMES.len());
    }

    #[test]
    fn a_browser_given_must_exist() {
        let missing = Path::new("/no/such/browser");
        let err = find(Some(missing)).unwrap_err().to_string();
        assert!(err.contains("--browser"), "{err}");
    }

    #[test]
    fn the_browser_never_holds_a_gpui_agent_token() {
        for headed in [false, true] {
            let command = command(Path::new("chrome"), headed, Path::new("/tmp/profile"));
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == "GPUI_AGENT_TOKEN" && value.is_none()),
                "the browser inherits no token (headed: {headed})"
            );
            assert_eq!(command.get_args().any(|arg| arg == "--headless"), !headed);
        }
    }
}
