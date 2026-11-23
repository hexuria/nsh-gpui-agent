//! Builds the runtime Java bridge into dex bytecode for Android targets.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=java/com/longbridge/nsh/agent/AgentBridge.java");
    println!("cargo:rerun-if-env-changed=ANDROID_HOME");
    println!("cargo:rerun-if-env-changed=ANDROID_SDK_ROOT");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");
    println!("cargo:rerun-if-env-changed=LOCALAPPDATA");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }
    let sdk = sdk_root().expect("Android SDK: set ANDROID_HOME or ANDROID_SDK_ROOT");
    let android_jar = newest(&sdk.join("platforms"), "android-")
        .map(|dir| dir.join("android.jar"))
        .filter(|jar| jar.is_file())
        .expect("an Android platform android.jar");
    let d8_names: &[&str] = if cfg!(windows) {
        &["d8.bat", "d8"]
    } else {
        &["d8"]
    };
    let d8 = newest(&sdk.join("build-tools"), "")
        .and_then(|dir| {
            d8_names
                .iter()
                .map(|name| dir.join(name))
                .find(|d8| d8.is_file())
        })
        .expect("d8 in the Android SDK build-tools");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let classes = out.join("java-classes");
    if classes.exists() {
        fs::remove_dir_all(&classes).expect("remove stale Java classes");
    }
    fs::create_dir_all(&classes).expect("create Java classes directory");
    let javac_name = if cfg!(windows) { "javac.exe" } else { "javac" };
    let javac = env::var_os("JAVA_HOME")
        .map(|home| PathBuf::from(home).join("bin").join(javac_name))
        .filter(|javac| javac.is_file())
        .unwrap_or_else(|| PathBuf::from(javac_name));
    run(
        Command::new(javac)
            .arg("-cp")
            .arg(&android_jar)
            .arg("-d")
            .arg(&classes)
            .arg("java/com/longbridge/nsh/agent/AgentBridge.java"),
        "javac",
    );
    let mut inputs = Vec::new();
    class_files(&classes, &mut inputs);
    run(
        program_command(&d8)
            .arg("--min-api")
            .arg("26")
            .arg("--lib")
            .arg(android_jar)
            .arg("--output")
            .arg(&out)
            .args(&inputs),
        "d8",
    );
    assert!(out.join("classes.dex").is_file(), "d8 wrote no classes.dex");
}

/// Finds the Android SDK the way NSH's command line does: its environment
/// first, then each platform's usual installation directory.
fn sdk_root() -> Option<PathBuf> {
    env::var_os("ANDROID_HOME")
        .or_else(|| env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(windows) {
                env::var_os("LOCALAPPDATA")
                    .map(|dir| PathBuf::from(dir).join("Android/Sdk"))
                    .filter(|path| path.is_dir())
            } else {
                None
            }
        })
        .or_else(|| {
            env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Android/Sdk"))
                .filter(|path| path.is_dir())
        })
        .or_else(|| {
            env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library/Android/sdk"))
                .filter(|path| path.is_dir())
        })
}

/// Runs a build tool directly, or through cmd.exe when it is a Windows batch
/// file (`.bat` is not itself an executable image there).
fn program_command(program: &Path) -> Command {
    let batch = cfg!(windows)
        && program
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bat"));
    if batch {
        let mut command = Command::new("cmd.exe");
        command.arg("/C").arg(program);
        command
    } else {
        Command::new(program)
    }
}

/// The lexicographically newest matching directory, which sorts Android's
/// numeric-only directory names correctly except across digit widths; prefer
/// a numeric comparison when both parse as numbers.
fn newest(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
        })
        .collect();
    entries.sort_by(|a, b| {
        let number = |path: &PathBuf| {
            path.file_name()
                .map(|name| {
                    name.to_string_lossy()
                        .trim_start_matches(prefix)
                        .split('.')
                        .next()
                        .and_then(|part| part.parse::<u32>().ok())
                        .unwrap_or(0)
                })
                .unwrap_or(0)
        };
        number(a).cmp(&number(b)).then_with(|| a.cmp(b))
    });
    entries.pop()
}

fn class_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read Java classes") {
        let path = entry.expect("read Java class entry").path();
        if path.is_dir() {
            class_files(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "class")
        {
            files.push(path);
        }
    }
}

fn run(command: &mut Command, name: &str) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("run {name}: {error}"));
    assert!(
        output.status.success(),
        "{name} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
