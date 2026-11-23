# nsh-gpui-agent

Makes an app built with [Native Shell](https://github.com/longbridge/nsh) (NSH) drivable by
[gpui-agent](https://github.com/hexuria/gpui-agent): a client can read the app's tree of views, press, type, scroll
and take screenshots, over a loopback connection the app opens only when it is told to.

> **Status: moving.** The adapter and the agent actions are here and their headless tests pass against the commit
> below. Windows has been run from this repository; until the platforms table says otherwise, another platform does
> not work from here. The plan is in
> [`hexuria/nsh`, branch `feat/nsh-agent-socket`](https://github.com/hexuria/nsh/tree/feat/nsh-agent-socket/handover/agent-socket).

## What is here

| Crate | What it does |
|---|---|
| `crates/nsh-agent-actions` | The strict agent actions: a semantic tree of the mounted views, and node and point actions (press, long press, type, scroll, keys, keybindings, screenshots) that prove their target with a native hit-test before acting. Hosts plug in through two traits, `SemanticScene` (implemented here for NSH's `nsh_testing::agent::Model`) and `StrictNative` (each platform implements it). |
| `crates/nsh-gpui-agent` | The gpui-agent adapter: serves `gpui-agent`'s protocol over loopback and maps its requests onto `nsh-agent-actions`. |
| `crates/nsh-gpui-agent-windows` | Attaches the adapter to Windows development surfaces and builds a development host with it installed. |

## Which NSH to use

**Use `https://github.com/hexuria/nsh`, branch `gpui-agent`.** Upstream NSH on its own does not work.

| | |
|---|---|
| Repository | `https://github.com/hexuria/nsh` |
| Branch | `gpui-agent` |
| What it is | An upstream release of `longbridge/nsh` plus a small set of doors, and nothing else |
| Upstream release it sits on | `v0.6.1` (`2f51d8611`) |
| Commit this repository is tested with | [`56add34a0`](https://github.com/hexuria/nsh/commit/56add34a019022de6433dcf6fa62b345d33a3342) |
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
# nsh-core, nsh-dev and nsh-testing are the nsh-* crates this repository depends on
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
| Windows | run; resize caveat below | Windows Server 2022, `hexuria/nsh` `56add34a0`, `gpui-agent-cli` 0.3.0 |
| Android | not run yet | |
| iOS | not run yet | |
| Web | not run yet | |

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

## License

Apache-2.0, as NSH and gpui-agent are.
