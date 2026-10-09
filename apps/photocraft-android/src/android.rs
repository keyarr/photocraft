//! Android entry point: the one isolated `unsafe` item in this crate (see the
//! `[lints.rust]` note in `Cargo.toml`).
//!
//! `android-activity` declares `extern "Rust" { fn android_main(..) }`, so our
//! definition must keep the literal symbol name or the `.so` won't link.

#![allow(unsafe_code)]

// SAFETY: `#[unsafe(no_mangle)]` only fixes the symbol name so the
// android-activity glue (C, in that crate) resolves `android_main`; it does
// not make the body unsafe. The body calls safe Rust only (logger init and
// `crate::run`), takes no raw pointers, and returns `()` on `Destroy` via
// winit's normal shutdown path.
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    log::info!("photocraft-android: activity start");
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    if let Err(e) = crate::run(options) {
        log::error!("photocraft-android: run_native failed: {e}");
    }
}
