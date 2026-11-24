# nsh-gpui-agent

Makes an app built with [Native Shell](https://github.com/longbridge/nsh) (NSH) drivable by
[gpui-agent](https://github.com/hexuria/gpui-agent): a client can read the app's tree of views, press, type, scroll
and take screenshots, over a loopback connection the app opens only when it is told to.

[![CI](https://github.com/hexuria/nsh-gpui-agent/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/hexuria/nsh-gpui-agent/actions/workflows/ci.yml)

Everything that makes an NSH app drivable lives here: the adapter, the agent actions and one crate per platform.
NSH itself keeps only a few small doors. Nothing is published to crates.io: depend on a commit of this repository.

## What is here

| Crate | What it does |
|---|---|
| `crates/nsh-agent-actions` | The strict agent actions: a semantic tree of the mounted views, and node and point actions (press, long press, type, scroll, keys, keybindings, screenshots) that prove their target with a native hit-test before acting. Hosts plug in through two traits, `SemanticScene` (implemented here for NSH's `nsh_testing::agent::Model`) and `StrictNative` (each platform implements it). |
| `crates/nsh-gpui-agent` | The gpui-agent adapter: serves `gpui-agent`'s protocol over loopback and maps its requests onto `nsh-agent-actions`. |
| `crates/nsh-gpui-agent-ios-host` | A development host that registers the iOS surface hook before starting NSH, supplied through `NSH_RUNTIME`. |
| `crates/nsh-gpui-agent-ios` | A Simulator launcher wrapping `nsh_cli::main()`, with a private one-use token handoff. |
| `crates/nsh-agent-web` | An NSH CLI wrapper for owned-Chromium or user-opened Web tabs, with page-side hooks and synthetic tab input. |
| `crates/nsh-gpui-agent-linux` | The Linux surface hook, strict native driver and opt-in `nsh-linux-host` development binary. |
| `crates/nsh-gpui-agent-windows` | Attaches the adapter to Windows development surfaces and builds a development host with it installed. |
| `crates/nsh-gpui-agent-android` | Android: attaches to a development surface, loads its Java bridge from dex bytecode compiled while the crate builds, and implements `StrictNative` for the agent. |
| `crates/nsh-gpui-agent-cli` | The `nsh-gpui-agent` command line: NSH's command line with the plug's shared development-host launch hook, including the Android token handoff. |
| `crates/nsh-gpui-agent-macos` | The macOS surface hook, strict native driver, in-process server and opt-in `nsh-macos-host` development binary. |

## Which NSH to use

**Use the pinned commit below from `https://github.com/hexuria/nsh`, branch `gpui-agent`.**
Upstream NSH on its own does not have the required doors.

| | |
|---|---|
| Repository | `https://github.com/hexuria/nsh` |
| Branch | `gpui-agent` |
| What it is | An upstream release of `longbridge/nsh` plus a small set of doors, and nothing else |
| Upstream release it sits on | `v0.6.1` (`2f51d8611`) |
| Dependency pin (`Cargo.toml`, every `nsh-*` crate) | [`365864ba8`](https://github.com/hexuria/nsh/commit/365864ba8a3e56189f9a4fcdaf1c874313fc028e) |
| The doors | `SOCKET.md` at the root of that branch lists every one |

A *door* is a small change that makes something in NSH public: an accessor, or a hook that calls out. A door holds
no logic. This repository needs a few of them, and upstream NSH does not have them yet, which is the only reason a
fork is involved. If upstream takes the doors, this section changes to "use upstream NSH".

The branch is rebased onto each new upstream release, so depend on a commit, not on the branch name:

```toml
[dependencies]
nsh-testing = { git = "https://github.com/hexuria/nsh", rev = "365864ba8a3e56189f9a4fcdaf1c874313fc028e", default-features = false, features = ["agent"] }
```

## Using it from an app

A build can have only one NSH. Cargo treats the same crate from two places as two different crates. The small crate
that links a platform plug must therefore also depend directly on one NSH crate at the pinned commit; NSH's generated
workspace reads that direct dependency to learn which NSH checkout the generated hosts must share. Use the same
checkout for both entries:

```toml
# the linking crate's Cargo.toml
[dependencies]
# A relative path while testing this clone, or git + rev for a pinned commit.
nsh-gpui-agent-android = { path = "../nsh-gpui-agent/crates/nsh-gpui-agent-android" }
nsh-testing = { git = "https://github.com/hexuria/nsh", rev = "365864ba8a3e56189f9a4fcdaf1c874313fc028e", default-features = false, features = ["agent"] }
```

Do not omit the direct `nsh-testing` dependency merely because the crate's Rust code does not name it: without a
direct NSH dependency the generated host falls back to crates.io and its component types no longer match the plug's.

An app that builds NSH from its own checkout instead of the pinned commit must make that checkout the one source in
two places. Change the direct dependency to it:

```toml
nsh-testing = { path = "../nsh/crates/nsh-testing", default-features = false, features = ["agent"] }
```

and redirect this repository's transitive git dependencies in the application workspace's root `Cargo.toml` (a
`[patch]` table in a dependency or non-root member is ignored):

```toml
# the application workspace root Cargo.toml
[patch."https://github.com/hexuria/nsh"]
nsh-core    = { path = "../nsh/crates/nsh-core" }
nsh-testing = { path = "../nsh/crates/nsh-testing" }
nsh-dev     = { path = "../nsh/crates/nsh-dev" }
nsh-base = { path = "../nsh/crates/nsh-base" }
nsh-platform-ios = { path = "../nsh/crates/nsh-platform-ios" } # when enabling ios
nsh-platform-linux = { path = "../nsh/crates/nsh-platform-linux" }
nsh-linux-host = { path = "../nsh/crates/nsh-linux-host" }
nsh-platform-android = { path = "../nsh/crates/nsh-platform-android" }
nsh-platform-macos = { path = "../nsh/crates/nsh-platform-macos" }
nsh-macos-host = { path = "../nsh/crates/nsh-macos-host" }
# Include this when linking the Web CLI wrapper:
nsh-cli     = { path = "../nsh/crates/nsh-cli" }
# plus every other nsh-* crate this repository's dependency graph resolves from
# the git source — check `cargo tree -i nsh-testing` (and the other nsh-* crates)
# has exactly one instance each; a second copy silently splits global state
# (the surface hook never fires).
```

The same patch is needed in every Cargo workspace that compiles a host with the
agent: generated workspaces (for example an app's `nsh/.nsh/rust/Cargo.toml`)
are separate workspaces, so the app's own patch does not reach them. Add the
same `[patch."https://github.com/hexuria/nsh"]` block there after each build
regenerates the manifest.

The app's NSH must contain every door branch `gpui-agent` has at the pinned commit. Our own app line, `hexuria/nsh`
branch `buwiz-next`, is that branch plus the app's other changes (at `0cfc41294` it has every door of the pin above). It is built by replaying the doors, not by
merging, so it is not a descendant of the pin: compare it with `SOCKET.md` each time the pin moves. A door the app's
NSH lacks shows up as `no method named ...` in one of this repository's platform crates, compiled against the app's
checkout.

## An app with its own Rust host

`NSH_RUNTIME` (the sections below) is for apps that use NSH's stock development host. An app that has its own Rust
host links the plug into it instead. This is how eBIRForms (`hexuria/buwiz-forms`, directory `nsh/`) does it:

- **One dependency per platform, all at the same commit of this repository.** Each goes under its
  `[target.'cfg(target_os = "...")'.dependencies]`, optional, behind one feature of the app. Different commits
  would link the shared crates (`nsh-gpui-agent`, `nsh-agent-actions`) twice.
- **Install before the first surface opens.** Call `nsh_gpui_agent_macos::install()`,
  `nsh_gpui_agent_linux::install()`, `nsh_gpui_agent_windows::install()` or `nsh_gpui_agent::ios::install()` at the
  top of the host's entry point. Linking alone does nothing. Android is the exception: `nsh-gpui-agent-android`
  installs itself when the library loads.
- **The `[patch]` above, in the generated workspace too.** NSH rewrites `nsh/.nsh/rust/Cargo.toml` on every build,
  so the app re-adds the block each time. eBIRForms does it from the script it sets as `CARGO` for agent builds.
- **A port of its own for any second listener.** The plug serves the window (tree, press, type, screenshot) on
  `GPUI_AGENT_ADDR`. An app that also serves a gpui-agent session of its own must put it elsewhere; eBIRForms uses
  the next port.

Nothing starts without `GPUI_AGENT=1`, a loopback `GPUI_AGENT_ADDR` and a token.

## Linux development host

Install the GTK/WebKitGTK prerequisites in NSH's `docs/dev/linux.md` (Ubuntu 24.04 or equivalent).
Build this repository's host, not the one in the NSH checkout:

```sh
cargo build -p nsh-gpui-agent-linux --bin nsh-linux-host
mkdir -p target/nsh-runtime/linux
ln -sf "$PWD/target/debug/nsh-linux-host" "$PWD/target/nsh-runtime/linux/nsh-linux-host"
export NSH_RUNTIME="$PWD/target/nsh-runtime"
export GPUI_AGENT_TOKEN="$(openssl rand -hex 32)"
export GPUI_AGENT_ADDR=127.0.0.1:17421
GPUI_AGENT=1 xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
  nsh run ../nsh/examples/gallery --target linux
```

Use the same address and token with `gpui-agent` in another terminal. An app with its own Rust host takes
precedence over `NSH_RUNTIME`: link `nsh-gpui-agent-linux` into that host and call `install()` before opening
surfaces. The hook retains its worker until the surface closes; without the opt-in environment no server starts.
The ordinary NSH host, Linux native test driver and `nsh test` path are unchanged.

The Linux move was exercised under Xvfb with the same buwiz gallery source and fresh app storage on both hosts:
hello, snapshots, click, typing/Backspace (including native focus preservation), rejected F12, menu shortcut,
press, hover, scroll, drag, viewport resize and PNG capture. The final pinned replay matched all 23 JSON files
byte-for-byte; each PNG differed only in 20 pixels on the input caret's one-pixel-wide column. An earlier replay
caught a different point in the initial navigation transition; its later snapshots matched. `invoke focus` is unsupported on both hosts; type/key exercise
the native focus operation. Ordinary gallery tests also expose upstream native-only limitations (for example
`audio()` is headless-only); the old `agent-*` fixtures use colon-containing selectors that upstream rejects.

### Android

List a small crate of the app in `nsh.json` `"rust"`. The crate depends on `nsh-gpui-agent-android` and re-exports
its native-module entry so NSH's generated Android library links it:

```rust
pub use nsh_gpui_agent_android::__nsh_native_module;
```

```json
"rust": "native/agent"
```

The crate needs a Java JDK and the Android SDK while it builds for Android: its build script compiles the bridge with
`javac` and `d8` (NSH's minimum SDK is 26, so the runtime uses `InMemoryDexClassLoader`). Set `JAVA_HOME` or put
`javac` on `PATH`, and set `ANDROID_HOME` or `ANDROID_SDK_ROOT`. Without those variables the build script looks in
`~/Android/Sdk` on Linux, `~/Library/Android/sdk` on macOS and `%LOCALAPPDATA%\\Android\\Sdk` on Windows; on Windows it
uses `javac.exe` and runs `d8.bat` through `cmd.exe`.

Build the command line once from this clone and use it instead of the stock `nsh` binary:

```sh
cargo install --path crates/nsh-gpui-agent-cli --locked
```

With `GPUI_AGENT=1` it requires `GPUI_AGENT_TOKEN` (no surrounding whitespace) and accepts a loopback
`GPUI_AGENT_ADDR` (default `127.0.0.1:17421`); the token goes to the development host's private files over adb's
standard input and is never an argument. On PowerShell, a run looks like:

```powershell
$env:GPUI_AGENT = "1"
$env:GPUI_AGENT_TOKEN = "<one random token>"
nsh-gpui-agent run --target android path\to\the\app
adb forward tcp:17421 tcp:17421
Remove-Item Env:GPUI_AGENT, Env:GPUI_AGENT_TOKEN
```

Use a fresh random token for each machine or run; do not commit it.

## macOS development host

Build this repository's host, not the one in the NSH checkout, and put it where `nsh run` looks for a runtime.
Use an `nsh` CLI built from the same doors commit:

```sh
cargo build -p nsh-gpui-agent-macos --bin nsh-macos-host
mkdir -p target/nsh-runtime/macos
ln -sf "$PWD/target/debug/nsh-macos-host" "$PWD/target/nsh-runtime/macos/nsh-macos-host"
export NSH_RUNTIME="$PWD/target/nsh-runtime"
export GPUI_AGENT_TOKEN="$(openssl rand -hex 32)"
export GPUI_AGENT_ADDR=127.0.0.1:17421
GPUI_AGENT=1 nsh run ../nsh/examples/gallery --target macos
```

Use the same address and token with `gpui-agent` in another terminal. An app with its own Rust host takes precedence
over `NSH_RUNTIME`: link `nsh-gpui-agent-macos` into that host and call `install()` before opening surfaces. The hook
retains its worker until the surface closes; without the opt-in environment no server starts. Feature `gpui-agent`
exposes the in-process tree/action API (`AppViewAgentExt`); feature `gpui-agent-server` also compiles the legacy
in-process listener. `dev-agent` is on by default here and starts the strict adapter only in debug builds; when
`gpui-agent-server` is also enabled, the private server owns the listener and no strict worker starts, matching
buwiz's feature split. A release embedding that wants only the in-process API or server should disable default
features so the development adapter is not linked.

One build-time difference remains: because the doors are dev-gated, `nsh-platform-macos` is pinned with its `dev`
feature even when a consumer selects only the in-process server. That compiles upstream development support into such
a build. Input provenance, settle behavior, file drops, surface ownership and window lookup match buwiz.

## gpui-agent version

This repository speaks `gpui-agent` `=0.3.0`, the version it was moved with, so behavior could be compared like for
like. Moving to 0.4.0 is the next step and is not done.

An app may link another gpui-agent version for a listener of its own. eBIRForms links 0.4.0 for its data listener
beside this repository's 0.3.0 window listener, and cargo builds both. Both answered `protocol: 2` in the run of
2026-11-24. Until the move, use the 0.3.0 command line with the window listener, as the platform sections say.

## Keybindings and app windows

Added on 2026-11-23 with the doors of the same day:

- **A keybinding runs by its id.** `keybinding` fires the exact binding its id names (`keymap.<n>.<chord>`)
  through each platform's key-command door (macOS, Linux, Windows, iOS), so a chord two bindings share is no
  longer ambiguous.
- **A request aimed at one of the app's own windows stays in that window** on Linux and Windows: hit tests, the
  focus check and resizing use the window doors. The main window's path is unchanged. macOS already aimed at
  windows; iOS has no app windows.

Driven live in a real app on macOS and Linux, on the main window. No live run has aimed at a second app window
yet; on Linux and Windows that path is covered by the build, the tests and the lint in CI.

## Platforms

### Web development

Build the workspace and install the matching client. A source-built wrapper needs a matching
Web runtime, just as a distributed NSH CLI does.

Before building that runtime, install the wasm target on **every OS**. Run from the pinned
fork checkout: its `rust-toolchain.toml` selects the `stable` channel (not a fixed Rust version).

```sh
rustup target add --toolchain stable wasm32-unknown-unknown
```

The runtime's C dependencies also need a wasm32-capable clang and LLVM archiver. Linux needed
clang/LLVM installed for our checks. On Windows, do not rely on the system C compiler: select
a clang and `llvm-ar` that support wasm32 explicitly. For example, use the Android NDK's LLVM
tools, or the official LLVM Windows release (`C:\Program Files\LLVM\bin`):

```powershell
# In the pinned fork checkout, using its stable toolchain:
rustup target add --toolchain stable wasm32-unknown-unknown
$llvmBin = "C:\path\to\android-ndk\toolchains\llvm\prebuilt\windows-x86_64\bin"
# Alternatively: $llvmBin = "C:\Program Files\LLVM\bin"
$env:CC_wasm32_unknown_unknown = Join-Path $llvmBin "clang.exe"
$env:AR_wasm32_unknown_unknown = Join-Path $llvmBin "llvm-ar.exe"
# Keep these variables set while running the Web build/test.
```

After a normal Web build/test in the pinned fork checkout, stage it from there:

```sh
mkdir -p /path/to/runtime/web
cp -R crates/nsh-platform-web/web/. /path/to/runtime/web/
cp target/wasm32-unknown-unknown/web/nsh-dev.wasm /path/to/runtime/web/
cp target/wasm32-unknown-unknown/web/nsh-syntaxes.bin /path/to/runtime/web/
export NSH_RUNTIME=/path/to/runtime
```

`NSH_RUNTIME` is the CLI's existing runtime-root override. Alternatively install the runtime
beside the wrapper at `../runtime/web`. Then, from this repository:

```sh
cargo build --workspace
cargo install gpui-agent-cli --version '=0.3.0' --locked
export GPUI_AGENT_TOKEN="$(openssl rand -hex 32)" GPUI_AGENT_ADDR=127.0.0.1:17421
# Run from an NSH checkout so examples/gallery is available.
GPUI_AGENT=1 /path/to/nsh-gpui-agent/target/debug/nsh-agent-web run examples/gallery --target web
# In another terminal with the same token and address:
gpui-agent wait
gpui-agent --reply-timeout-ms 90000 snapshot --pretty
gpui-agent screenshot --out gallery.png
```

With `GPUI_AGENT` unset the wrapper runs the ordinary NSH CLI. Chromium is found on `PATH`
(Chrome for Testing works); nothing downloads a browser. The default is an owned headless
browser at 1280×800. `GPUI_AGENT_WEB_HEADED=1` shows it in a window.

For a tab opened by hand, set `GPUI_AGENT_WEB_TAB=1` alongside `GPUI_AGENT=1` and open the
printed pairing URL locally. Its fragment is removed and the key is kept in session storage
for reloads. Only matching keys from loopback peers pair. Tab input is synthetic DOM input;
paste, screenshots, viewport changes and browser controls unavailable in buwiz remain unavailable.
Rust-root Web apps are not supported in agent mode. The token and address stay in the CLI
environment and are not passed to Chromium. Page hooks mask secure input reads; the plug's
semantic reader masks secure values from raw screen snapshots.

Page tests: `node --test crates/nsh-agent-web/src/page.test.mjs`.

The gallery's large initial tree exceeded the client's default 8-second reply timeout in the
Linux run; the example gives it a longer timeout.

Filled in from real runs on each platform's own machine. Headless tests on Linux do not count.

| Platform | Status | Tested on |
|---|---|---|
| macOS | Native strict-driver and shared-listener checks pass in a desktop session; gallery replay not run yet | arm64 Mac, macOS 26.6.2 |
| Linux | Gallery action parity verified: 23/23 JSON files identical; PNG differences confined to the input caret. Build the plug's `nsh-linux-host` and select it with `NSH_RUNTIME` as above. | Ubuntu 24.04 container on Ubuntu 22.04 host, Xvfb; GTK 4.14.5 / WebKitGTK 2.52.6 |
| Windows | run; resize caveat below | Windows Server 2022, `hexuria/nsh` `56add34a0`, `gpui-agent-cli` 0.3.0 |
| Android | passed the live agent run (hello through screenshot); 7/7 ordinary `button.test.tsx` tests passed | API 35 x86_64 emulator (Android 15) |
| iOS | Simulator attachment live-verified; native regression failures remain | iPhone 17, iOS 26.5, Apple Silicon; pinned fork above |
| Web | owned Chromium and independently opened tab: hello, snapshot, click and type; owned screenshot | Linux, Chrome for Testing 156.0.8078.47, gpui-agent-cli 0.3.0 |

### iOS integration

Enable this crate's `ios` feature and call `nsh_gpui_agent::ios::install()` before
the development host starts. A custom host can then call `nsh_ios_host::main()`.
The hook receives `nsh_platform_ios::dev::DevelopmentSurface` and retains one
mobile adapter worker and model for each opted-in session. Native input uses
NSH's existing driver; liveness, focus and native hit-testing are implemented
here. Ordinary `nsh test` keeps its separate model and driver.

The simulator-only, debug-only opt-in and private one-use token file are the same
as on `buwiz`: `GPUI_AGENT=1`, a loopback `GPUI_AGENT_ADDR`, and
`Library/Caches/gpui-agent-token` in the app's data container. Screenshots stay
confined to `Library/Caches/gpui-agent`; use `screenshot --inline` to save on the Mac.

For a TSX app using the base registries, build the supplied host and launcher:

```sh
rustup show active-toolchain
rustup target add aarch64-apple-ios-sim
cargo build --locked -p nsh-gpui-agent-ios-host --target aarch64-apple-ios-sim
cargo build --locked -p nsh-gpui-agent-ios
mkdir -p "$HOME/.nsh-agent-runtime/ios/iphonesimulator"
cp target/aarch64-apple-ios-sim/debug/nsh-ios-host "$HOME/.nsh-agent-runtime/ios/iphonesimulator/"
cmp target/aarch64-apple-ios-sim/debug/nsh-ios-host "$HOME/.nsh-agent-runtime/ios/iphonesimulator/nsh-ios-host"
export GPUI_AGENT_TOKEN="$(openssl rand -hex 32)" GPUI_AGENT_ADDR=127.0.0.1:17421
GPUI_AGENT=1 target/debug/nsh-gpui-agent-ios \
  --runtime "$HOME/.nsh-agent-runtime" --device SIMULATOR_UDID \
  --entry /path/to/nsh/examples/gallery
```

Use a booted Simulator's UDID (`xcrun simctl list devices booted`). Run the
matching **gpui-agent 0.3.0 CLI** in another terminal with the same address and
token. Install it into a dedicated directory and use that directory's binary:

```sh
cargo install gpui-agent-cli --version "=0.3.0" --root "$HOME/.gpui-agent-0.3.0"
export PATH="$HOME/.gpui-agent-0.3.0/bin:$PATH"
```

Older preinstalled CLIs can lack `press`, `scroll`, `long-press`, and
`screenshot --inline`; use the matching CLI rather than an older one on `PATH`.
Install the Simulator target on the **toolchain actually used by the build**:
run the target command above from this checkout, or specify the same toolchain
explicitly with `rustup target add --toolchain stable aarch64-apple-ios-sim`
and `cargo +stable build …`. The external buwiz-forms `scripts/setup-macos.sh`
runs its target installation under that app's Rust 1.99.0 override; it does not
also install the target into a separate `stable` toolchain. Building with
another toolchain without its target produces E0463 (`can't find crate for std`).
If `CARGO_TARGET_DIR` is set, use its build-output path instead of `target/`.
**After every host rebuild, repeat the copy and `cmp` above**: the launcher loads
`ios/iphonesimulator/nsh-ios-host` from `--runtime`, not directly from Cargo's
output. `cmp` must exit zero (and print nothing) before launching.

Bare `GPUI_AGENT=1 nsh run` still uses the stock host and does not link this plug.
Rust UI-set apps must link and install the plug in their own host registration;
the supplied base-only host does not add their crates to a generated host.

The launcher bootstraps the same development bundle ID to obtain the data
container, creates the token with mode `0600` without overwriting an existing
file, then calls the shared CLI library in a child process. Only the opt-in and
loopback address reach `simctl`'s app environment. The host consumes and deletes
the token before binding; launcher teardown and interrupt handling remove any
unconsumed token. The bootstrap install is an extra install compared with
`buwiz`, not an extra fork door. Startup failures are replayed through the
connected development client and the host's existing error overlay. As in
`buwiz`, subsequent bundle handling can dismiss that overlay.

Verified operations: authenticated hello/wait, snapshot, click/press, type,
Backspace, a Shift+A chord, set-value, scroll, drag, swipe, long-press dispatch,
back navigation, and native
PNG screenshots (including inline transfer). On the same iPhone 17 / iOS 26.5,
the gallery's Gestures long-press handler adds the same press-event text as
`buwiz`; both replies are `delivered_unverified`. The gallery home has no
keybindings on either host; its Button page exposes `alt-cmd-f`, which toggles
favorites on both. Before the first bundle mounts, a live coordinate press
returns exactly `native surface is unavailable` on both. Phones do not
advertise hover or viewport resizing.

Semantic IDs can go stale across navigation: after a click navigates, take a fresh
snapshot before choosing the next ID, or a stale ID can return `target_gone`. Offscreen
nodes, including duplicate header rows with zero-size bounds, can fail `press`
with `offscreen (it has no native view)`; choose the on-screen ID with visible
bounds. The gallery persists its last-open demo across launcher runs, so it can
reopen on Input rather than the index. Navigate to the intended page before
comparing snapshot node counts; a restored demo is not the initial gallery page.

The observer receives the same `Model::engine_started` notification as on
`buwiz`, so its mounted scene includes the whole page rather than only the
rows made near the viewport. The initial gallery snapshot contains 713 nodes
on both hosts. This notification completes the existing generic lifecycle
door; no agent policy was added to the fork.

**Stage 1 is not complete:** existing iOS native fixture tests have failures,
including keyboard focus, large-title geometry and selection tracking. All
eight failing cases also failed with a stock host from `089981575`, without the
plug. Ten consecutive `set-value` requests on existing input text replaced
8 times and appended 2 times on `buwiz`, versus 9 replacements and 1 append
on the plug. The shared intermittent behavior is recorded, not repaired as
part of this move. Plug tests, plug clippy (including the real Simulator target),
and the fork's prescribed iOS Catalyst clippy command passed. Detailed command
output and screenshots from this machine are under `~/stage1-ios/`; the
same-Simulator comparison is under `~/stage1-ios/parity/`.

### Windows

The gallery was run on a real Windows Server 2022 machine with this repository's development host. The ready
handshake, semantic snapshots, clicking a demo, typing into search and an account field, a key chord (`ctrl+a`),
a menu keybinding (opening the Quit confirmation), resizing, native PNG screenshots and a missing-target error
were exercised. Typing returned `delivered_verified`; the key chord returned success without a tree-visible
selection change (`delivered_unverified`). The menu confirmation and typed form were checked in a screenshot.

Resizing requested **1000×700** but produced **1000×731**. The host delegates resize to NSH's existing native
driver; `buwiz`'s client-area correction was not moved.

Build the host from `crates/nsh-gpui-agent-windows` and place it in a runtime root. From this repository, in
PowerShell:

```powershell
cargo build -p nsh-gpui-agent-windows --bin nsh-windows-host
$runtime = "$HOME\nsh-runtime"
New-Item -ItemType Directory -Force "$runtime\windows" | Out-Null
Copy-Item target\debug\nsh-windows-host.exe "$runtime\windows\nsh-windows-host.exe"
$env:NSH_RUNTIME = $runtime
$env:GPUI_AGENT = "1"
$env:GPUI_AGENT_ADDR = "127.0.0.1:17421"
$env:GPUI_AGENT_TOKEN = [guid]::NewGuid().ToString("N")
```

Then, from the `hexuria/nsh` checkout at the commit above, in the same shell:

```powershell
nsh run examples/gallery --target windows
```

`NSH_RUNTIME` selects `<root>/windows/nsh-windows-host.exe` instead of NSH's ordinary development host. The
gallery must have no `rust` host configuration, which takes precedence over `NSH_RUNTIME`. The adapter starts
only with `GPUI_AGENT=1`, a loopback address and a nonempty token.

Install the client with `cargo install gpui-agent-cli --version =0.3.0` (the `gpui-agent` package has no binary).
Run it in another shell with the same `GPUI_AGENT_ADDR` and `GPUI_AGENT_TOKEN`. For a screenshot saved to an
absolute local path, use `gpui-agent screenshot --inline --out <path.png>`; without `--inline`, the output name
must be relative to the host's screenshot directory. Native screenshots worked in this run despite the CLI help's
generic Windows limitation.

## How it is checked

Continuous integration runs on every push to `main` and every pull request. Each platform crate is an empty crate on
the wrong system, so each is built on its own:

| Job | System | What it runs |
|---|---|---|
| Shared crates and web | Ubuntu 24.04 | `cargo fmt --check`; the tests of `nsh-agent-actions` and `nsh-gpui-agent`; the web driver's tests; the page script's tests |
| Linux and Android | Ubuntu 24.04 | check, test and clippy of `nsh-gpui-agent-linux`; a check of `nsh-gpui-agent-android` for `aarch64-linux-android`, which compiles its Java bridge |
| macOS and iOS | macOS 15 | check, test and clippy of `nsh-gpui-agent-macos`; checks of the iOS driver, the launcher, and the host for the Simulator |
| Windows | Windows Server 2025 | check, test and clippy of `nsh-gpui-agent-windows` |

The badge at the top shows the last run on `main`.

CI opens no window. Two things need a desktop session and are run by hand:

- **The macOS native tests:** `cargo test -p nsh-gpui-agent-macos --test native -- --ignored`. Last run on
  2026-11-24 on an arm64 Mac: pass. Two of them report that key-window changes cannot be observed when the session
  is not in front.
- **Driving a real app.** The runs so far, with eBIRForms (`hexuria/buwiz-forms`):
  - 2026-11-24, macOS, this repository at `f97522d`: both listeners came up; hello, snapshot, a click that
    navigated, typing with the value read back, a screenshot, and a keybinding by id.
  - 2026-11-24, Linux under Xvfb, the same commit: hello, snapshot, a press that navigated, typing with the value
    read back, a viewport resize, a screenshot at the new size, and a key. The 0.3.0 and the 0.4.0 command lines
    were each accepted by both listeners.
  - 2026-11-23, this repository at `1e85cf65`, before the key-command and app-window doors: macOS, Linux, Windows,
    the iOS Simulator and an Android emulator, reported by the sessions that did the move.

## Where this came from

This code was moved out of the `hexuria/nsh` fork in November 2026, so that the fork carries only doors. The plan
it followed is in
[`hexuria/nsh`, branch `feat/nsh-agent-socket`](https://github.com/hexuria/nsh/tree/feat/nsh-agent-socket/handover/agent-socket).

## License

Apache-2.0, as NSH and gpui-agent are.
