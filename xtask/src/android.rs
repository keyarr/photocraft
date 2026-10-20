//! Android builds: `cargo xtask android <build|install|log|devices> [--release]`.
//!
//! The phone shell (`apps/niveta-android`) packages with `x` (xbuild):
//! same Rust, `android_main` instead of `main`, APK out. This wraps the
//! fiddly parts (env checks, APK discovery, `adb install`, filtered logcat)
//! so the dev loop is one command. When `x` itself fails, its output is the
//! error: this does not reinterpret it.

use std::path::PathBuf;
use std::process::Command;

use crate::{root, run};

const USAGE: &str = "\
usage: cargo xtask android <command> [--release]

commands:
  build [--release]   x build --platform android --arch arm64 --format apk
  install [--release] build, then `adb install -r` the freshest APK onto the
                      attached device (`adb devices` must list one)
  log                 `adb logcat` with the rust log of the running app
                      (tag niveta-android; Ctrl-C to stop)
  devices             `adb devices` plus what xbuild sees

env: ANDROID_HOME (or ANDROID_SDK_ROOT) must point at the SDK;
     the NDK inside it and the aarch64-linux-android rust target must exist.
     `x` (xbuild) must be on PATH (`cargo install xbuild`).";

/// `apps/niveta-android`, from the workspace root.
fn app_dir() -> PathBuf {
    root().join("apps").join("niveta-android")
}

/// ANDROID_HOME, or ANDROID_SDK_ROOT when the former is unset.
fn sdk() -> Option<PathBuf> {
    std::env::var_os("ANDROID_HOME").or_else(|| std::env::var_os("ANDROID_SDK_ROOT")).map(PathBuf::from)
}

/// The NDK's LLVM `bin` dir (newest NDK under the SDK): home of `clang` and
/// `aarch64-linux-android<api>-clang`, so the link works with no
/// machine-specific cargo config or PATH setup.
fn ndk_bin(sdk: &std::path::Path) -> Option<PathBuf> {
    let latest = std::fs::read_dir(sdk.join("ndk")).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).max()?;
    Some(latest.join("toolchains").join("llvm").join("prebuilt").join("linux-x86_64").join("bin"))
}

/// The NDK's `aarch64-linux-android<api>-clang` with the highest API level,
/// so the link works with no machine-specific cargo config.
fn android_linker(sdk: &std::path::Path) -> Option<PathBuf> {
    let bin = ndk_bin(sdk)?;
    std::fs::read_dir(&bin)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.starts_with("aarch64-linux-android") && n.ends_with("-clang") && !n.contains("++")
            })
        })
        .max()
}

/// The newest debug/release `.apk` xbuild left behind, if any: under the app
/// dir for a standalone package, under the workspace `target/x` when the app
/// belongs to a workspace (xbuild keeps its own output dir there).
fn freshest_apk(release: bool) -> Option<PathBuf> {
    let want = if release { "release" } else { "debug" };
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut stack = vec![app_dir().join("target"), root().join("target")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "apk") {
                let profile = p.to_string_lossy();
                if !profile.contains(want) {
                    continue;
                }
                let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                    best = Some((modified, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn need_x() -> Result<(), String> {
    if Command::new("x").arg("--version").output().is_ok_and(|o| o.status.success()) {
        return Ok(());
    }
    Err("`x` (xbuild) is not on PATH: `cargo install xbuild`, then re-run".into())
}

/// SDK, NDK and rust target: fail with what to install before `x` does.
fn need_toolchain() -> Result<PathBuf, String> {
    need_x()?;
    let sdk = sdk().ok_or("set ANDROID_HOME (or ANDROID_SDK_ROOT) to the SDK dir")?;
    if !sdk.join("platform-tools").join("adb").exists() {
        return Err(format!("no platform-tools/adb under {}", sdk.display()));
    }
    let ndk = sdk.join("ndk");
    if std::fs::read_dir(&ndk).into_iter().flat_map(|r| r.flatten()).next().is_none() {
        return Err(format!("no NDK under {}: `sdkmanager --install \"ndk;<ver>\"`", ndk.display()));
    }
    let out = Command::new("rustup").args(["target", "list", "--installed"]).output().map_err(|e| format!("rustup: {e}"))?;
    if !String::from_utf8_lossy(&out.stdout).lines().any(|l| l.split_whitespace().next() == Some("aarch64-linux-android")) {
        return Err("rust target aarch64-linux-android is missing: `rustup target add aarch64-linux-android`".into());
    }
    Ok(sdk)
}

fn cmd_build(release: bool) -> Result<(), String> {
    let sdk = need_toolchain()?;
    let mut c = Command::new("x");
    c.current_dir(app_dir()).arg("build").args(["--platform", "android", "--arch", "arm64", "--format", "apk"]);
    if let Some(linker) = android_linker(&sdk) {
        // xbuild shells out to cargo for the aarch64 .so but leaves the NDK
        // off PATH and pins a bare `clang` linker; without both the link
        // fails with `linker `clang` not found`.
        c.env("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER", linker);
    }
    if let Some(bin) = ndk_bin(&sdk) {
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        c.env("PATH", path);
    }
    if release {
        c.arg("--release");
    } else {
        c.arg("--debug");
    }
    run(c, if release { "x build android arm64 apk (release)" } else { "x build android arm64 apk (debug)" })?;
    match freshest_apk(release) {
        Some(p) => {
            println!("apk: {}", p.display());
            Ok(())
        }
        None => Err("x build succeeded but no APK found under apps/niveta-android/target".into()),
    }
}

fn cmd_install(release: bool) -> Result<(), String> {
    cmd_build(release)?;
    let sdk = need_toolchain()?;
    let apk = freshest_apk(release).ok_or("no APK to install")?;
    let mut c = Command::new(sdk.join("platform-tools").join("adb"));
    c.args(["install", "-r", &apk.to_string_lossy()]);
    run(c, "adb install -r <apk>")
}

fn cmd_log() -> Result<(), String> {
    let sdk = need_toolchain()?;
    // The rust log of the running app; everything else is noise.
    let mut c = Command::new(sdk.join("platform-tools").join("adb"));
    c.args(["logcat", "niveta-android:I", "*:S"]);
    run(c, "adb logcat niveta-android:I *:S")
}

fn cmd_devices() -> Result<(), String> {
    let sdk = need_toolchain()?;
    let mut adb = Command::new(sdk.join("platform-tools").join("adb"));
    adb.arg("devices");
    run(adb, "adb devices")?;
    run(Command::new("x"), "x devices")
}

pub fn cmd(args: &[&str]) -> Result<(), String> {
    let release = args.contains(&"--release");
    match args.first() {
        Some(cmd) => match *cmd {
            "build" => cmd_build(release),
            "install" => cmd_install(release),
            "log" => cmd_log(),
            "devices" => cmd_devices(),
            other => Err(format!("unknown android command `{other}`\n\n{USAGE}")),
        },
        None => {
            print!("{USAGE}");
            Ok(())
        }
    }
}
