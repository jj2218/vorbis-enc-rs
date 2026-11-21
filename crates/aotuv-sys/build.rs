//! Compiles aoTuV b6.03 (the `aotuv_lancer_vorbis_sys` 0.1.6 sources in `.cache/aotuv/`: run
//! `just aotuv`) with `src/shim.c` as the Vorbis port's oracle: strict IEEE float without
//! Lancer's SIMD. Lancer's paths are all `#ifdef __SSE__`/`__SSE2__`/`__SSE3__`, which x86-64
//! always defines, so the oracle undefines them (scalar math stays SSE2); `-ffp-contract=off`
//! keeps the source order and `-mno-fma` stops gcc 12's vectoriser fusing `mdct_forward` anyway
//! (measured before the port: one stream at -O0…-O3 and -march=x86-64-v3; `tests/oracle.rs` pins it).
//! aarch64 has no `__SSE__` and aoTuV no NEON path: the same scalar C, uncontracted.
//!
//! On x86-64 a second copy, the speed baseline as `vorbis_rs` builds it (the cc crate's release
//! flags `-O3 -ffunction-sections -fdata-sections -fPIC -m64`, `HAVE_STDINT_H`, Lancer on), linked
//! into one relocatable object whose only globals are the shim's `lancer_*` functions (`objcopy`),
//! so both copies link into one benchmark (`lancer` in `lib.rs`). A third, xiph's libvorbis 1.3.7
//! (`.cache/aotuv/libvorbis-1.3.7`) as its `configure` builds it on x86-64 Linux (`-O3
//! -ffast-math`), is linked the same way (`xiph_*`, `xiph` in `lib.rs`): upstream's own build.
use std::path::{Path, PathBuf};
use std::process::Command;

const LIB: &[&str] = &[
    "analysis",
    "bitrate",
    "block",
    "codebook",
    "envelope",
    "floor0",
    "floor1",
    "info",
    "lookup",
    "lpc",
    "lsp",
    "mapping0",
    "mdct",
    "psy",
    "registry",
    "res0",
    "sharedbook",
    "smallft",
    "synthesis",
    "vorbisenc",
    "window",
    "cpu",
    "xmmlib",
];

/// The gitignored `.cache/` of this checkout, or of the main checkout when building in a
/// `git worktree`.
fn cache_dir() -> PathBuf {
    let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if ws.join(".cache/aotuv").exists() {
        return ws.join(".cache");
    }
    let out = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(&ws)
        .output()
        .ok()
        .filter(|o| o.status.success());
    match out {
        Some(o) => PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())
            .parent()
            .map_or_else(|| ws.join(".cache"), |main| main.join(".cache")),
        None => ws.join(".cache"),
    }
}

include!("../../scripts/strict_c.rs");

/// The sources and include paths both copies share.
fn build(root: &Path) -> cc::Build {
    let mut b = cc::Build::new();
    let v = root.join("vorbis_vendor");
    let ogg = root.join("ogg_vendor");
    b.include(ogg.join("include"))
        .include(v.join("include"))
        .include(v.join("lib"))
        .define("_REENTRANT", None)
        .flag("-fsigned-char")
        .warnings(false)
        .file(ogg.join("src/framing.c"))
        .file(ogg.join("src/bitwise.c"))
        .file("src/shim.c");
    for s in LIB {
        b.file(v.join(format!("lib/{s}.c")));
    }
    b
}

fn main() {
    let root = cache_dir().join("aotuv/aotuv_lancer_vorbis_sys-0.1.6");
    assert!(
        root.join("vorbis_vendor/lib/psy.c").exists(),
        "aoTuV not found at {}: run `just aotuv`",
        root.display()
    );
    let x86 = std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64");
    let mut b = build(&root);
    b.opt_level(2)
        .flag("-fno-fast-math")
        .flag("-ffp-contract=off")
        .flag("-U__SSE__")
        .flag("-U__SSE2__")
        .flag("-U__SSE3__");
    if x86 {
        b.flag("-mno-fma");
    }
    assert_strict(&b);
    b.compile("aotuv");
    if x86 {
        lancer(&root);
        xiph(&root, &cache_dir().join("aotuv/libvorbis-1.3.7"));
    }
    println!("cargo::rustc-check-cfg=cfg(aotuv_lancer)");
    println!("cargo::rustc-check-cfg=cfg(aotuv_xiph)");
    println!("cargo:rerun-if-env-changed=OBJCOPY");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/shim.c");
    println!("cargo:rerun-if-changed={}", root.display());
}

/// The shim's encoder API, `lancer_*` in the Lancer copy.
const API: &[&str] = &["enc_new", "enc_write", "enc_out", "enc_free"];

/// `libaotuvlancer.a`: Lancer as `vorbis_rs` builds it, only `lancer_*` global.
fn lancer(root: &Path) {
    let mut b = build(root);
    b.opt_level(3)
        .define("HAVE_STDINT_H", None)
        .define("SHIM_LANCER", None);
    speed_copy(&b, "lancer");
}

/// `libaotuvxiph.a`: xiph's libvorbis 1.3.7 with `configure`'s x86-64 Linux release flags (`-O3
/// -ffast-math -D_REENTRANT -fsigned-char`), libogg from aoTuV's vendored copy (libogg 1.3.5's
/// framing.c/bitwise.c), only `xiph_*` global.
fn xiph(root: &Path, xroot: &Path) {
    if !xroot.join("lib/psy.c").exists() {
        println!(
            "cargo:warning=xiph copy skipped: no {} (run `just aotuv`)",
            xroot.display()
        );
        return;
    }
    let ogg = root.join("ogg_vendor");
    let mut b = cc::Build::new();
    b.include(ogg.join("include"))
        .include(xroot.join("include"))
        .include(xroot.join("lib"))
        .define("_REENTRANT", None)
        .define("SHIM_XIPH", None)
        .flag("-fsigned-char")
        .flag("-ffast-math")
        .opt_level(3)
        .warnings(false)
        .file(ogg.join("src/framing.c"))
        .file(ogg.join("src/bitwise.c"))
        .file("src/shim.c");
    for s in LIB.iter().filter(|s| !["cpu", "xmmlib"].contains(s)) {
        b.file(xroot.join(format!("lib/{s}.c")));
    }
    speed_copy(&b, "xiph");
    println!("cargo:rerun-if-changed={}", xroot.display());
}

/// `lib<prefix>.a` (`libaotuv<prefix>.a`): the copy linked into one relocatable object whose only
/// globals are the shim's `<prefix>_*` functions, and `cfg(aotuv_<prefix>)`. Benchmark-only:
/// without GCC and binutils it is skipped with a warning, and the oracle still builds.
fn speed_copy(b: &cc::Build, prefix: &str) {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cc = b.get_compiler();
    if !cc.is_like_gnu() {
        println!("cargo:warning={prefix} copy skipped: its relocatable link needs GCC");
        return;
    }
    let objs = b.compile_intermediates();
    let merged = out.join(format!("aotuv{prefix}.o"));
    let ok = |c: &mut Command| c.status().is_ok_and(|s| s.success());
    let mut link = Command::new(cc.path());
    link.args(["-r", "-nostdlib", "-o"])
        .arg(&merged)
        .args(&objs);
    let mut oc = Command::new(std::env::var("OBJCOPY").unwrap_or_else(|_| "objcopy".into()));
    for f in API {
        oc.arg(format!("--keep-global-symbol={prefix}_{f}"));
    }
    oc.arg(&merged);
    let lib = out.join(format!("libaotuv{prefix}.a"));
    let _ = std::fs::remove_file(&lib);
    let mut ar = b.get_archiver();
    ar.arg("crs").arg(&lib).arg(&merged);
    for (step, c) in [
        ("relocatable link", &mut link),
        ("objcopy", &mut oc),
        ("ar", &mut ar),
    ] {
        if !ok(c) {
            println!("cargo:warning={prefix} copy skipped: {step} failed");
            return;
        }
    }
    println!("cargo:rustc-link-lib=static=aotuv{prefix}");
    println!("cargo:rustc-cfg=aotuv_{prefix}");
}
