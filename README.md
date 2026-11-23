# nsh-gpui-agent

Makes an app built with [Native Shell](https://github.com/longbridge/nsh) (NSH) drivable by
[gpui-agent](https://github.com/hexuria/gpui-agent): a client can read the app's tree of views, press, type, scroll
and take screenshots, over a loopback connection the app opens only when it is told to.

> **Status: moving.** The adapter and the agent actions are here and their headless tests pass against the commit
> below. The iOS Simulator attachment runs from this repository, but its native regression suite is not all green.
> See the platforms table for what has actually been checked. The plan is in
> [`hexuria/nsh`, branch `feat/nsh-agent-socket`](https://github.com/hexuria/nsh/tree/feat/nsh-agent-socket/handover/agent-socket).

## What is here

| Crate | What it does |
|---|---|
| `crates/nsh-agent-actions` | The strict agent actions: a semantic tree of the mounted views, and node and point actions (press, long press, type, scroll, keys, keybindings, screenshots) that prove their target with a native hit-test before acting. Hosts plug in through two traits, `SemanticScene` (implemented here for NSH's `nsh_testing::agent::Model`) and `StrictNative` (each platform implements it). |
| `crates/nsh-gpui-agent` | The gpui-agent adapter: serves `gpui-agent`'s protocol over loopback and maps its requests onto `nsh-agent-actions`. |
| `crates/nsh-gpui-agent-ios-host` | A development host that registers the iOS surface hook before starting NSH, supplied through `NSH_RUNTIME`. |
| `crates/nsh-gpui-agent-ios` | A Simulator launcher wrapping `nsh_cli::main()`, with a private one-use token handoff. |

## Which NSH to use

**Use `https://github.com/hexuria/nsh`, branch `gpui-agent`.** Upstream NSH on its own does not work.

| | |
|---|---|
| Repository | `https://github.com/hexuria/nsh` |
| Branch | `gpui-agent` |
| What it is | An upstream release of `longbridge/nsh` plus a small set of doors, and nothing else |
| Upstream release it sits on | `v0.6.1` (`2f51d8611`) |
| Dependency pin (iOS Simulator tested) | [`56add34a0`](https://github.com/hexuria/nsh/commit/56add34a019022de6433dcf6fa62b345d33a3342) |
| Last Stage 0 headless test run | [`56add34a0`](https://github.com/hexuria/nsh/commit/56add34a019022de6433dcf6fa62b345d33a3342) |
| The doors | `SOCKET.md` at the root of that branch lists every one |

A *door* is a small change that makes something in NSH public: an accessor, or a hook that calls out. A door holds
no logic. This repository needs a few of them, and upstream NSH does not have them yet, which is the only reason a
fork is involved. If upstream takes the doors, this section changes to "use upstream NSH".

The branch is rebased onto each new upstream release, so depend on a commit, not on the branch name:

```toml
[dependencies]
nsh-testing = { git = "https://github.com/hexuria/nsh", rev = "56add34a019022de6433dcf6fa62b345d33a3342", default-features = false, features = ["agent"] }
```

## Using it from an app

A build can have only one NSH. Cargo treats the same crate from two places as two different crates, so an app that
builds NSH from its own checkout must point this repository's NSH at that same checkout:

```toml
# the app's Cargo.toml
[patch."https://github.com/hexuria/nsh"]
nsh-core    = { path = "../nsh/crates/nsh-core" }
nsh-testing = { path = "../nsh/crates/nsh-testing" }
nsh-dev     = { path = "../nsh/crates/nsh-dev" }
nsh-platform-ios = { path = "../nsh/crates/nsh-platform-ios" } # when enabling ios
```

The app's NSH must contain everything branch `gpui-agent` has. Our own app line, `hexuria/nsh` branch `buwiz-next`,
is that branch plus the app's other changes.

## gpui-agent version

`gpui-agent` `=0.3.0` during the move, so behavior can be compared like for like. Moving to 0.4.0 is the plan's last
stage.

## Platforms

Filled in from real runs on each platform's own machine. Headless tests on Linux do not count.

| Platform | Status | Tested on |
|---|---|---|
| macOS | not run yet | |
| Linux | not run yet | |
| Windows | not run yet | |
| Android | not run yet | |
| iOS | Simulator attachment live-verified; native regression failures remain | iPhone 17, iOS 26.5, Apple Silicon; pinned fork above |
| Web | not run yet | |

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

## License

Apache-2.0, as NSH and gpui-agent are.
