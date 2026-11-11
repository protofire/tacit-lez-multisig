use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

const RAPIDSNARK_DOWNLOAD_SCRIPT: &str = include_str!("./download_rapidsnark.sh");
const RAPIDSNARK_LIB_DIR_ENV_VAR: &str = "RAPIDSNARK_LIB_DIR";
const RAPIDSNARK_VERSION_FILE: &str = "../RAPIDSNARK_VERSION";

fn main() {
    let target = env::var("TARGET").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    let arch = target.split('-').next().unwrap();

    // See: https://github.com/zkmopro/chkstk_stub
    chkstk_stub::build();

    println!("cargo:rerun-if-changed={RAPIDSNARK_VERSION_FILE}");
    println!("cargo:rerun-if-env-changed={RAPIDSNARK_LIB_DIR_ENV_VAR}");

    // module build: flake.nix unpacks the fetchurl'd release libs here (no curl in the cargo build env)
    let prebuilt = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../prebuilt");
    let rapidsnark_lib_dir = if prebuilt.exists() {
        prebuilt
    } else if let Ok(lib_dir) = env::var(RAPIDSNARK_LIB_DIR_ENV_VAR) {
        println!("cargo:warning=rapidsnark: using pre-supplied libs from {RAPIDSNARK_LIB_DIR_ENV_VAR}={lib_dir}");
        Path::new(&lib_dir).to_path_buf()
    } else {
        println!("cargo:warning=rapidsnark: {RAPIDSNARK_LIB_DIR_ENV_VAR} not set, downloading...");
        provide_rapidsnark(&out_dir, arch)
    }
    .canonicalize()
    .unwrap_or_else(|error| panic!("{RAPIDSNARK_LIB_DIR_ENV_VAR} is not a valid path: {error}"));

    let compiler = cc::Build::new().get_compiler();
    let cpp_stdlib = if compiler.is_like_clang() {
        "c++"
    } else {
        "stdc++"
    };

    println!(
        "cargo:rustc-link-search=native={}",
        rapidsnark_lib_dir.display()
    );

    // The shared rapidsnark artifact is already linked with its fr/fq/gmp implementation.
    // Linking those helper libraries again is redundant and can accidentally pull static archives
    // into downstream binaries, exporting generic Fr_* / Fq_* symbols that collide with other
    // native ZK libraries.
    //
    // The static rapidsnark archive is different: it leaves Fr_*, Fq_*, and GMP symbols unresolved,
    // so static/mobile builds must link the helper archives.
    if is_static_rapidsnark() || is_mobile_target() {
        println!("cargo:rustc-link-lib=static=rapidsnark");
        println!("cargo:rustc-link-lib=static=fr");
        println!("cargo:rustc-link-lib=static=fq");
        println!("cargo:rustc-link-lib=static=gmp");
    } else {
        println!("cargo:rustc-link-lib=dylib=rapidsnark");
    }

    println!("cargo:rustc-link-lib={cpp_stdlib}");

    // Android bundles pthread into libc
    let thread_lib = if is_android_target() { "c" } else { "pthread" };
    println!("cargo:rustc-link-lib={thread_lib}");
}

fn rapidsnark_version() -> String {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let version_file = Path::new(&manifest_dir).join(RAPIDSNARK_VERSION_FILE);
    fs::read_to_string(&version_file)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", version_file.display()))
        .trim()
        .to_string()
}

fn provide_rapidsnark(out_dir: &str, arch: &str) -> std::path::PathBuf {
    let rapidsnark_path = Path::new(out_dir).join("rapidsnark");
    if !is_rapidsnark_downloaded(&rapidsnark_path, arch) {
        let version = rapidsnark_version();
        let rapidsnark_script_path = Path::new(out_dir).join("download_rapidsnark.sh");
        fs::write(&rapidsnark_script_path, RAPIDSNARK_DOWNLOAD_SCRIPT)
            .expect("Failed to write build script");
        let child_process = Command::new("sh")
            .arg(rapidsnark_script_path.to_str().unwrap())
            .env("RAPIDSNARK_VERSION", &version)
            .spawn();
        if let Err(e) = child_process {
            panic!("Failed to spawn rapidsnark download: {e}");
        }
        let status = child_process.unwrap().wait();
        if let Err(e) = status {
            panic!("Failed to wait for rapidsnark download: {e}");
        } else if !status.unwrap().success() {
            panic!("Failed to wait for rapidsnark download");
        }
    }
    rapidsnark_path.join(arch)
}

/// Checks whether the arch-specific subdirectory exists, not just the parent directory.
///
/// The download script creates the parent via `mkdir -p` before fetching, so checking the parent
/// would silently skip re-downloads after a previously interrupted build.
///
/// The `arch` subdirectory (e.g. `x86_64/`) is itself unnecessary for isolation: `OUT_DIR` is
/// already scoped to the full target triple by Cargo. A cleaner future refactor (option B) would
/// have the script copy libs directly into `$OUT_DIR/rapidsnark/lib/` and sentinel on the presence
/// of `librapidsnark.a` — eliminating the extra directory level.
fn is_rapidsnark_downloaded(rapidsnark_path: &Path, arch: &str) -> bool {
    rapidsnark_path.join(arch).exists()
}

fn is_static_rapidsnark() -> bool {
    env::var_os("CARGO_FEATURE_STATIC_RAPIDSNARK").is_some()
}

fn is_mobile_target() -> bool {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    target_os.contains("ios") || target_os.contains("android")
}

fn is_android_target() -> bool {
    env::var("CARGO_CFG_TARGET_OS").unwrap().contains("android")
}
