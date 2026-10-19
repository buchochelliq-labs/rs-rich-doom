//! Compiles the vendored doomgeneric engine (GPL-2.0) and the C side of
//! this port (csrc/) with the `cc` crate.

use std::fs;

/// The engine's framebuffer: Doom's own 320x200, so a frame is one byte of
/// work per pixel the engine drew, scaled on the Rust side.
const RESX: u32 = 320;
const RESY: u32 = 200;

fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let engine_dir = root.join("doomgeneric");
    let shim_dir = root.join("csrc");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_family = std::env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    let sound = std::env::var_os("CARGO_FEATURE_SOUND").is_some();

    let mut sources: Vec<_> = fs::read_dir(&engine_dir)
        .expect("doomgeneric/ is vendored with the crate")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect();
    sources.sort();

    let define = |build: &mut cc::Build| {
        build
            .include(&engine_dir)
            .define("DOOMGENERIC_RESX", RESX.to_string().as_str())
            .define("DOOMGENERIC_RESY", RESY.to_string().as_str())
            .define("RS_RICH_DOOM", None)
            .define("_DEFAULT_SOURCE", None)
            .define("NORMALUNIX", None)
            .warnings(false)
            .opt_level(2);
        if target_os == "linux" {
            build.define("LINUX", None);
        }
        if sound {
            build.define("FEATURE_SOUND", None);
        }
    };

    // The engine, with its console output and exit() sent to Rust.
    let mut engine = cc::Build::new();
    define(&mut engine);
    let redirect = shim_dir.join("redirect.h");
    let compiler = engine.get_compiler();
    if compiler.is_like_msvc() {
        engine.flag(format!("/FI{}", redirect.display()));
    } else {
        engine
            .flag("-include")
            .flag(redirect.to_str().expect("a UTF-8 path"))
            // 1990s C: implicit declarations and loose pointer types.
            .flag_if_supported("-fno-strict-aliasing")
            .flag_if_supported("-Wno-implicit-function-declaration")
            .flag_if_supported("-Wno-int-conversion")
            .flag_if_supported("-Wno-incompatible-pointer-types");
    }
    engine.files(&sources).compile("doomgeneric");

    // The platform layer, without the redirect (it is what the redirect
    // calls).
    let mut shim = cc::Build::new();
    define(&mut shim);
    shim.file(shim_dir.join("shim.c"));
    if sound {
        shim.file(shim_dir.join("sound.c"));
    }
    shim.compile("rs_rich_doom_shim");

    println!("cargo:rerun-if-changed=doomgeneric");
    println!("cargo:rerun-if-changed=csrc");
    println!("cargo:rerun-if-changed=build.rs");
    if target_family == "unix" && target_os != "macos" {
        println!("cargo:rustc-link-lib=m");
    }
}
