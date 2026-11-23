# nsh-gpui-agent

Makes an app built with [Native Shell](https://github.com/longbridge/nsh) (NSH) drivable by
[gpui-agent](https://github.com/hexuria/gpui-agent): a client can read the app's tree of views, press, type, scroll
and take screenshots, over a loopback connection the app opens only when it is told to.

> **Status: moving.** The adapter and the agent actions are here and their headless tests pass against the commit
> below. macOS's native checks pass; its end-to-end gallery replay has not run yet. Until the platforms table says
> otherwise, a platform does not work from here. The plan is in
> [`hexuria/nsh`, branch `feat/nsh-agent-socket`](https://github.com/hexuria/nsh/tree/feat/nsh-agent-socket/handover/agent-socket).

## What is here

| Crate | What it does |
|---|---|
| `crates/nsh-agent-actions` | The strict agent actions: a semantic tree of the mounted views, and node and point actions (press, long press, type, scroll, keys, keybindings, screenshots) that prove their target with a native hit-test before acting. Hosts plug in through two traits, `SemanticScene` (implemented here for NSH's `nsh_testing::agent::Model`) and `StrictNative` (each platform implements it). |
| `crates/nsh-gpui-agent` | The gpui-agent adapter: serves `gpui-agent`'s protocol over loopback and maps its requests onto `nsh-agent-actions`. |
| `crates/nsh-gpui-agent-macos` | The macOS surface hook, strict native driver, in-process server and opt-in `nsh-macos-host` development binary. |

## Which NSH to use

**Use the pinned commit below from `https://github.com/hexuria/nsh`.** The macOS doors are on
`devin/stage1-macos-doors` until merged into `gpui-agent`; upstream NSH on its own does not work.

| | |
|---|---|
| Repository | `https://github.com/hexuria/nsh` |
| Branch | `devin/stage1-macos-doors` (macOS doors on top of `gpui-agent`) |
| What it is | An upstream release of `longbridge/nsh` plus a small set of doors, and nothing else |
| Upstream release it sits on | `v0.6.1` (`2f51d8611`) |
| Commit this repository is tested with | [`542291b7a`](https://github.com/hexuria/nsh/commit/542291b7a3870824432dd56d60db28c2709825d7) |
| The doors | `SOCKET.md` at the root of that branch lists every one |

A *door* is a small change that makes something in NSH public: an accessor, or a hook that calls out. A door holds
no logic. This repository needs a few of them, and upstream NSH does not have them yet, which is the only reason a
fork is involved. If upstream takes the doors, this section changes to "use upstream NSH".

The branch is rebased onto each new upstream release, so depend on a commit, not on the branch name:

```toml
[dependencies]
nsh-testing = { git = "https://github.com/hexuria/nsh", rev = "542291b7a3870824432dd56d60db28c2709825d7", default-features = false, features = ["agent"] }
```

## Using it from an app

A build can have only one NSH. Cargo treats the same crate from two places as two different crates, so an app that
builds NSH from its own checkout must point this repository's NSH at that same checkout:

```toml
# the app's Cargo.toml
[patch."https://github.com/hexuria/nsh"]
nsh-core           = { path = "../nsh/crates/nsh-core" }
nsh-testing        = { path = "../nsh/crates/nsh-testing" }
nsh-dev            = { path = "../nsh/crates/nsh-dev" }
nsh-base           = { path = "../nsh/crates/nsh-base" }
nsh-platform-macos = { path = "../nsh/crates/nsh-platform-macos" }
nsh-macos-host     = { path = "../nsh/crates/nsh-macos-host" }
```

The app's NSH must contain everything branch `gpui-agent` has. Our own app line, `hexuria/nsh` branch `buwiz-next`,
is that branch plus the app's other changes.

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

`gpui-agent` `=0.3.0` during the move, so behavior can be compared like for like. Moving to 0.4.0 is the plan's last
stage.

## Platforms

Filled in from real runs on each platform's own machine. Headless tests on Linux do not count.

| Platform | Status | Tested on |
|---|---|---|
| macOS | Native strict-driver and shared-listener checks pass in a desktop session; gallery replay not run yet | arm64 Mac, macOS 26.6.2 |
| Linux | not run yet | |
| Windows | not run yet | |
| Android | not run yet | |
| iOS | not run yet | |
| Web | not run yet | |

## License

Apache-2.0, as NSH and gpui-agent are.
