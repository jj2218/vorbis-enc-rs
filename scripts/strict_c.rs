// Shared by the float C oracles' build scripts (`include!`d by crates/{aotuv,liblc3}-sys/build.rs).
//
// A strict-IEEE oracle is only an oracle if nothing overrides its float flags. The cc crate puts
// the user's CFLAGS / CFLAGS_<target> / TARGET_CFLAGS (and a `CC="gcc <flags>"` wrapper's flags)
// after the build script's, so the last `-ffast-math` wins and the oracle silently changes
// (measured: `CFLAGS=-ffast-math` fails aotuv-sys's `spike_hashes`). `assert_strict` panics on
// any final compiler argument that changes float results. `-mfma`/`-march=` stay allowed: with
// contraction pinned off (`-ffp-contract=off`, every other contraction mode rejected here), the C
// source never becomes an FMA, so the ISA flags only change speed; but they also select a C
// library's own SIMD code paths (liblc3's NEON kernels on aarch64): its build script checks that.

/// Fails the build if `b`'s final compiler command could change float results, or if the
/// target's float math is not the verified SSE2/AArch64 kind (x87 elsewhere is not strict).
fn assert_strict(b: &cc::Build) {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    assert!(
        arch == "x86_64" || arch == "aarch64",
        "strict-IEEE C oracle: target arch `{arch}` is unverified (x86_64 and aarch64 only: \
         elsewhere, e.g. x87 math, float results are not strict)"
    );
    let bad: Vec<String> = b
        .get_compiler()
        .to_command()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .filter(|a| changes_floats(a))
        .collect();
    assert!(
        bad.is_empty(),
        "strict-IEEE C oracle: {bad:?} would change its float results (they come from CFLAGS, \
         CFLAGS_<target>, TARGET_CFLAGS, the CC wrapper or RUSTFLAGS); unset them for this build"
    );
}

/// GCC/Clang flags that let the compiler reorder, contract, approximate or narrow float
/// arithmetic.
fn changes_floats(a: &str) -> bool {
    const EXACT: &[&str] = &[
        "-ffast-math",
        "-Ofast",
        "-funsafe-math-optimizations",
        "-fassociative-math",
        "-freciprocal-math",
        "-ffinite-math-only",
        "-fno-signed-zeros",
        "-fno-trapping-math",
        "-fcx-limited-range",
        "-fcx-fortran-rules",
        "-fsingle-precision-constant",
        "-fapprox-func",
        "-menable-unsafe-fp-math",
        "-mrecip",
        "-m32",
        "-m80387",
    ];
    EXACT.contains(&a)
        || a.starts_with("-mrecip=")
        || a.starts_with("-ffp-contract=") && a != "-ffp-contract=off"
        || a.starts_with("-ffp-model=") && a != "-ffp-model=strict"
        || a.starts_with("-mfpmath=") && a != "-mfpmath=sse"
}
