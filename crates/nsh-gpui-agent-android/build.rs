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
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }
    let sdk = env::var_os("ANDROID_HOME")
        .or_else(|| env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var("HOME").expect("HOME")).join("Android/Sdk"));
    let android_jar = newest(&sdk.join("platforms"), "android-")
        .map(|dir| dir.join("android.jar"))
        .filter(|jar| jar.is_file())
        .expect("an Android platform android.jar");
    let d8 = newest(&sdk.join("build-tools"), "")
        .map(|dir| dir.join("d8"))
        .filter(|d8| d8.is_file())
        .expect("d8 in the Android SDK build-tools");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let classes = out.join("java-classes");
    if classes.exists() {
        fs::remove_dir_all(&classes).expect("remove stale Java classes");
    }
    fs::create_dir_all(&classes).expect("create Java classes directory");
    let javac = env::var_os("JAVA_HOME")
        .map(|home| PathBuf::from(home).join("bin/javac"))
        .filter(|javac| javac.is_file())
        .unwrap_or_else(|| PathBuf::from("javac"));
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
        Command::new(d8)
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
