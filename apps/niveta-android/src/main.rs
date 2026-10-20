//! Host runner for the Android shell: `cargo run -p niveta-android`.
//! On a device the entry point is `android_main` in the library.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() -> eframe::Result {
    niveta_android::run(eframe::NativeOptions::default())
}
