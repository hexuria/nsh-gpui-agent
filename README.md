# nsh-gpui-agent

Makes an app built with [Native Shell](https://github.com/longbridge/nsh) (NSH) drivable by
[gpui-agent](https://github.com/hexuria/gpui-agent): a client can read the app's tree of views, press, type, scroll
and take screenshots, over a loopback connection the app opens only when it is told to.

> **Status: moving.** The adapter and the agent actions are here and their headless tests pass against the commit
> below. No platform has been run from this repository yet: until the platforms table says otherwise, a platform does
> not work from here. The plan is in
> [`hexuria/nsh`, branch `feat/nsh-agent-socket`](https://github.com/hexuria/nsh/tree/feat/nsh-agent-socket/handover/agent-socket).

## What is here

| Crate | What it does |
|---|---|
| `crates/nsh-agent-actions` | The strict agent actions: a semantic tree of the mounted views, and node and point actions (press, long press, type, scroll, keys, keybindings, screenshots) that prove their target with a native hit-test before acting. Hosts plug in through two traits, `SemanticScene` (implemented here for NSH's `nsh_testing::agent::Model`) and `StrictNative` (each platform implements it). |
| `crates/nsh-gpui-agent` | The gpui-agent adapter: serves `gpui-agent`'s protocol over loopback and maps its requests onto `nsh-agent-actions`. |
| `crates/nsh-gpui-agent-linux` | The Linux surface hook, strict native driver and opt-in `nsh-linux-host` development binary. |

## Which NSH to use

**Use the pinned commit below from `https://github.com/hexuria/nsh`.** The Linux doors are on
`devin/linux-agent-doors` until merged into `gpui-agent`; upstream NSH on its own does not work.

| | |
|---|---|
| Repository | `https://github.com/hexuria/nsh` |
| Branch | `devin/linux-agent-doors` (Linux doors on top of `gpui-agent`) |
| What it is | An upstream release of `longbridge/nsh` plus a small set of doors, and nothing else |
| Upstream release it sits on | `v0.6.1` (`2f51d8611`) |
| Commit this repository is tested with | [`c367eea75`](https://github.com/hexuria/nsh/commit/c367eea75cf708b550ce9dc04844cda2e318ad7d) |
| The doors | `SOCKET.md` at the root of that branch lists every one |

A *door* is a small change that makes something in NSH public: an accessor, or a hook that calls out. A door holds
no logic. This repository needs a few of them, and upstream NSH does not have them yet, which is the only reason a
fork is involved. If upstream takes the doors, this section changes to "use upstream NSH".

The branch is rebased onto each new upstream release, so depend on a commit, not on the branch name:

```toml
[dependencies]
nsh-testing = { git = "https://github.com/hexuria/nsh", rev = "c367eea75cf708b550ce9dc04844cda2e318ad7d", default-features = false, features = ["agent"] }
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
nsh-platform-linux = { path = "../nsh/crates/nsh-platform-linux" }
nsh-linux-host = { path = "../nsh/crates/nsh-linux-host" }
nsh-base = { path = "../nsh/crates/nsh-base" }
```

The app's NSH must contain everything branch `gpui-agent` has. Our own app line, `hexuria/nsh` branch `buwiz-next`,
is that branch plus the app's other changes.

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

## gpui-agent version

`gpui-agent` `=0.3.0` during the move, so behavior can be compared like for like. Moving to 0.4.0 is the plan's last
stage.

## Platforms

Filled in from real runs on each platform's own machine. Headless tests on Linux do not count.

| Platform | Status | Tested on |
|---|---|---|
| macOS | not run yet | |
| Linux | Xvfb gallery actions exercised; 23/23 JSON files identical, PNG differences confined to the input caret (above) | Ubuntu 24.04 sidecar, GTK 4.14.5 / WebKitGTK 2.52.6 |
| Windows | not run yet | |
| Android | not run yet | |
| iOS | not run yet | |
| Web | not run yet | |

## License

Apache-2.0, as NSH and gpui-agent are.
