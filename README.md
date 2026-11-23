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
| `crates/nsh-gpui-agent-android` | Android: attaches to a development surface, loads its Java bridge from dex bytecode compiled while the crate builds, and implements `StrictNative` for the agent. |
| `crates/nsh-gpui-agent-cli` | The `nsh-gpui-agent` command line: NSH's command line with the plug's shared development-host launch hook, including the Android token handoff. |

## Which NSH to use

**Use `https://github.com/hexuria/nsh`, branch `gpui-agent`.** Upstream NSH on its own does not work.

| | |
|---|---|
| Repository | `https://github.com/hexuria/nsh` |
| Branch | `gpui-agent` |
| What it is | An upstream release of `longbridge/nsh` plus a small set of doors, and nothing else |
| Upstream release it sits on | `v0.6.1` (`2f51d8611`) |
| Commit this repository is tested with | [`6c24cda40`](https://github.com/hexuria/nsh/commit/6c24cda40d4e88dbf4e55a2b97a680ccc0bc4094) |
| The doors | `SOCKET.md` at the root of that branch lists every one |

A *door* is a small change that makes something in NSH public: an accessor, or a hook that calls out. A door holds
no logic. This repository needs a few of them, and upstream NSH does not have them yet, which is the only reason a
fork is involved. If upstream takes the doors, this section changes to "use upstream NSH".

The branch is rebased onto each new upstream release, so depend on a commit, not on the branch name:

```toml
[dependencies]
nsh-testing = { git = "https://github.com/hexuria/nsh", rev = "6c24cda40d4e88dbf4e55a2b97a680ccc0bc4094", default-features = false, features = ["agent"] }
```

## Using it from an app

A build can have only one NSH. Cargo treats the same crate from two places as two different crates, so an app that
builds NSH from its own checkout must point this repository's NSH at that same checkout:

```toml
# the app's Cargo.toml
[patch."https://github.com/hexuria/nsh"]
nsh-cli               = { path = "../nsh/crates/nsh-cli" }
nsh-core              = { path = "../nsh/crates/nsh-core" }
nsh-dev               = { path = "../nsh/crates/nsh-dev" }
nsh-platform-android  = { path = "../nsh/crates/nsh-platform-android" }
nsh-testing           = { path = "../nsh/crates/nsh-testing" }
# These are the nsh-* crates this repository depends on directly.
```

The app's NSH must contain everything branch `gpui-agent` has. Our own app line, `hexuria/nsh` branch `buwiz-next`,
is that branch plus the app's other changes.

### Android

List a small crate of the app in `nsh.json` `"rust"`. The crate depends on `nsh-gpui-agent-android` and re-exports
its native-module entry so NSH's generated Android library links it:

```rust
pub use nsh_gpui_agent_android::__nsh_native_module;
```

```json
"rust": "native/agent"
```

The crate needs a Java JDK (`javac`) and the Android SDK while it builds for Android: its build script compiles the
bridge with `javac` and `d8` (NSH's minimum SDK is 26, so the runtime uses `InMemoryDexClassLoader`). Build and run
with the `nsh-gpui-agent` binary instead of the stock `nsh` binary. With `GPUI_AGENT=1` it requires
`GPUI_AGENT_TOKEN` (no surrounding whitespace) and accepts a loopback `GPUI_AGENT_ADDR` (default `127.0.0.1:17421`);
the token goes to the development host's private files over adb's standard input and is never an argument.

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
| Android | passed the live agent run (hello through screenshot); 7/7 ordinary `button.test.tsx` tests passed | API 35 x86_64 emulator (Android 15) |
| iOS | not run yet | |
| Web | not run yet | |

## License

Apache-2.0, as NSH and gpui-agent are.
