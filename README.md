# nsh-gpui-agent

Makes an app built with [Native Shell](https://github.com/longbridge/nsh) (NSH) drivable by
[gpui-agent](https://github.com/hexuria/gpui-agent): a client can read the app's tree of views, press, type, scroll
and take screenshots, over a loopback connection the app opens only when it is told to.

> **Status: moving.** The adapter and the agent actions are here and their headless tests pass against the commit
> below. Web has been exercised on Linux; until the platforms table says otherwise, another platform does
> not work from here. The plan is in
> [`hexuria/nsh`, branch `feat/nsh-agent-socket`](https://github.com/hexuria/nsh/tree/feat/nsh-agent-socket/handover/agent-socket).

## What is here

| Crate | What it does |
|---|---|
| `crates/nsh-agent-actions` | The strict agent actions: a semantic tree of the mounted views, and node and point actions (press, long press, type, scroll, keys, keybindings, screenshots) that prove their target with a native hit-test before acting. Hosts plug in through two traits, `SemanticScene` (implemented here for NSH's `nsh_testing::agent::Model`) and `StrictNative` (each platform implements it). |
| `crates/nsh-gpui-agent` | The gpui-agent adapter: serves `gpui-agent`'s protocol over loopback and maps its requests onto `nsh-agent-actions`. |
| `crates/nsh-agent-web` | An NSH CLI wrapper for owned-Chromium or user-opened Web tabs, with page-side hooks and synthetic tab input. |

## Which NSH to use

**Use `https://github.com/hexuria/nsh`, branch `gpui-agent`.** Upstream NSH on its own does not work.

| | |
|---|---|
| Repository | `https://github.com/hexuria/nsh` |
| Branch | `gpui-agent` |
| What it is | An upstream release of `longbridge/nsh` plus a small set of doors, and nothing else |
| Upstream release it sits on | `v0.6.1` (`2f51d8611`) |
| Commit this repository is tested with | [`2cce2a8d0`](https://github.com/hexuria/nsh/commit/2cce2a8d030495e4b20346a58afe23349f6cd6bf) |
| The doors | `SOCKET.md` at the root of that branch lists every one |

A *door* is a small change that makes something in NSH public: an accessor, or a hook that calls out. A door holds
no logic. This repository needs a few of them, and upstream NSH does not have them yet, which is the only reason a
fork is involved. If upstream takes the doors, this section changes to "use upstream NSH".

The branch is rebased onto each new upstream release, so depend on a commit, not on the branch name:

```toml
[dependencies]
nsh-testing = { git = "https://github.com/hexuria/nsh", rev = "2cce2a8d030495e4b20346a58afe23349f6cd6bf", default-features = false, features = ["agent"] }
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
# Include this when linking the Web CLI wrapper:
nsh-cli     = { path = "../nsh/crates/nsh-cli" }
```

The app's NSH must contain everything branch `gpui-agent` has. Our own app line, `hexuria/nsh` branch `buwiz-next`,
is that branch plus the app's other changes.

## gpui-agent version

`gpui-agent` `=0.3.0` during the move, so behavior can be compared like for like. Moving to 0.4.0 is the plan's last
stage.

## Platforms

### Web development

Build the workspace and install the matching client. A source-built wrapper needs a matching
Web runtime, just as a distributed NSH CLI does. After a normal Web build/test in the pinned
fork checkout, stage it from there:

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
| macOS | not run yet | |
| Linux | not run yet | |
| Windows | not run yet | |
| Android | not run yet | |
| iOS | not run yet | |
| Web | owned Chromium and independently opened tab: hello, snapshot, click and type; owned screenshot | Linux, Chrome for Testing 156.0.8078.47, gpui-agent-cli 0.3.0 |

## License

Apache-2.0, as NSH and gpui-agent are.
