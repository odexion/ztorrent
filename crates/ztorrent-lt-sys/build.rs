//! Compiles libtorrent-rasterbar and OpenSSL from source and links both
//! statically, so the app runs on machines that have neither installed.
//!
//! - The libtorrent release tarball is downloaded once into OUT_DIR and checked
//!   against a pinned SHA-256. LIBTORRENT_SRC points at an unpacked source tree
//!   instead, for offline builds.
//! - OpenSSL comes from the `openssl-src` crate (a 3.x release).
//! - Boost is header-only here. BOOST_INCLUDE (or BOOST_ROOT) points at its
//!   headers when they are not under a default prefix.
//! - ZTORRENT_SKIP_NATIVE=1 compiles no C++ at all, for `cargo check` against
//!   another platform's target. Nothing built that way can link.
//! - ZTORRENT_NATIVE_CACHE names a folder where compiled libtorrent and OpenSSL
//!   are kept between builds, keyed on everything that goes into them. A
//!   version bump reruns this script, which otherwise downloads and compiles
//!   both again.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

const LT_VERSION: &str = "2.1.1";
const LT_SHA256: &str = "0f163516ecef2e3331500266751de3098835a3c3ae0c2290448046c632bc0e93";

fn main() {
    for var in ["BOOST_INCLUDE", "BOOST_ROOT", "LIBTORRENT_SRC", "ZTORRENT_SKIP_NATIVE", "ZTORRENT_NATIVE_CACHE", "MACOSX_DEPLOYMENT_TARGET"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=src/bridge.cpp");
    println!("cargo:rerun-if-changed=include/bridge.h");
    if std::env::var("ZTORRENT_SKIP_NATIVE").as_deref() == Ok("1") {
        return;
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    let boost = boost_include();
    let cache = native_cache(&boost);
    let native = match cache.as_deref().filter(|dir| dir.join("ready").is_file()) {
        Some(dir) => Native::cached(dir),
        None => {
            let native = build_native(&out, &boost, &os, msvc);
            if let Some(dir) = &cache {
                if let Err(err) = native.store(dir) {
                    println!("cargo:warning=could not keep libtorrent and OpenSSL in {}: {err}", dir.display());
                }
            }
            native
        }
    };

    // The bridge first: with static archives the linker wants each library
    // before the ones it depends on.
    let mut bridge = cxx_build::bridge("src/lib.rs");
    bridge.file("src/bridge.cpp").std("c++17").includes(native.includes(&boost));
    for (key, value) in public_defines(&os) {
        bridge.define(key, value);
    }
    configure(&mut bridge, msvc);
    bridge.compile("ztorrent-lt-bridge");
    native.link();

    match os.as_str() {
        "macos" => {
            // ip_notifier and the default-route lookup.
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
            println!("cargo:rustc-link-lib=framework=SystemConfiguration");
        }
        "windows" => {
            for name in ["bcrypt", "mswsock", "ws2_32", "iphlpapi", "dbghelp", "crypt32", "user32", "advapi32"] {
                println!("cargo:rustc-link-lib={name}");
            }
        }
        _ => {
            println!("cargo:rustc-link-lib=pthread");
            println!("cargo:rustc-link-lib=dl");
        }
    }
}

/// Definitions the headers must see identically in libtorrent and in the
/// bridge, or the two disagree about the layout of the same types.
fn public_defines(os: &str) -> Vec<(&'static str, Option<&'static str>)> {
    let mut public = vec![
        ("TORRENT_ABI_VERSION", Some("2")),
        ("BOOST_ASIO_ENABLE_CANCELIO", None),
        ("BOOST_ASIO_NO_DEPRECATED", None),
        ("BOOST_SYSTEM_USE_UTF8", None),
        ("_SILENCE_CXX17_ALLOCATOR_VOID_DEPRECATION_WARNING", None),
        ("TORRENT_USE_OPENSSL", None),
        ("TORRENT_USE_LIBCRYPTO", None),
        ("TORRENT_SSL_PEERS", None),
        ("OPENSSL_NO_SSL2", None),
        ("OPENSSL_NO_SSL3", None),
        ("OPENSSL_NO_TLS1", None),
        ("OPENSSL_NO_TLS1_1", None),
        ("OPENSSL_NO_DTLS1", None),
        // WebRTC peers need libdatachannel; the Homebrew build ztorrent was
        // developed against had them off too.
        ("TORRENT_USE_RTC", Some("0")),
    ];
    if os == "windows" {
        public.extend([
            ("WIN32_LEAN_AND_MEAN", None),
            ("_WIN32_WINNT", Some("0x0A00")),
            ("BOOST_ALL_NO_LIB", None),
            ("_SCL_SECURE_NO_DEPRECATE", None),
            ("_CRT_SECURE_NO_DEPRECATE", None),
        ]);
    }
    public
}

/// Compiled libtorrent and OpenSSL: the headers the bridge is built against,
/// and the static libraries the app links.
struct Native {
    lt_include: PathBuf,
    lt_lib_dir: PathBuf,
    ssl_include: PathBuf,
    ssl_lib_dir: PathBuf,
    ssl_libs: Vec<String>,
}

/// libtorrent's archive, by the name `cc` gives it.
fn lt_archive(msvc: bool) -> &'static str {
    if msvc { "torrent-rasterbar.lib" } else { "libtorrent-rasterbar.a" }
}

fn build_native(out: &Path, boost: &Path, os: &str, msvc: bool) -> Native {
    let lt = libtorrent_source(out);
    let ssl = openssl_src::Build::new()
        // Where a static OpenSSL looks for trusted certificates at run time.
        // Windows needs none: libtorrent loads the system ROOT store itself. On
        // Linux the app also points SSL_CERT_FILE at the distribution's bundle.
        .openssl_dir(if os == "linux" { "/usr/lib/ssl" } else { "/etc/ssl" })
        .build();

    let cmake = std::fs::read_to_string(lt.join("CMakeLists.txt")).expect("libtorrent CMakeLists.txt");
    let mut lib = cc::Build::new();
    lib.cpp(true)
        .std("c++17")
        .includes([lt.join("include"), boost.to_path_buf(), ssl.include_dir().to_path_buf()])
        .include(lt.join("deps/try_signal"));
    for (list, dir) in [
        ("sources", "src"),
        ("kademlia_sources", "src/kademlia"),
        ("ed25519_sources", "src/ed25519"),
        ("try_signal_sources", "deps/try_signal"),
    ] {
        for file in cmake_list(&cmake, list) {
            lib.file(lt.join(dir).join(file));
        }
    }
    // Added outside the lists, when the encryption option is on (the default,
    // and what peers expect).
    lib.file(lt.join("src/pe_crypto.cpp"));
    for (key, value) in public_defines(os) {
        lib.define(key, value);
    }
    lib.define("TORRENT_BUILDING_LIBRARY", None)
        .define("BOOST_EXCEPTION_DISABLE", None)
        .define("BOOST_ASIO_HAS_STD_CHRONO", None)
        .define("NDEBUG", None);
    if os != "windows" {
        lib.define("_FILE_OFFSET_BITS", Some("64"));
    }
    // Hashing and piece bookkeeping are hot even in a debug build of the app,
    // and nobody steps through libtorrent: always optimise it.
    lib.opt_level(2).debug(false).warnings(false);
    configure(&mut lib, msvc);
    // Linked by `Native::link`, after the bridge that depends on it.
    lib.cargo_metadata(false).compile("torrent-rasterbar");

    Native {
        lt_include: lt.join("include"),
        lt_lib_dir: out.to_path_buf(),
        ssl_include: ssl.include_dir().to_path_buf(),
        ssl_lib_dir: ssl.lib_dir().to_path_buf(),
        ssl_libs: ssl.libs().to_vec(),
    }
}

impl Native {
    fn cached(dir: &Path) -> Native {
        let libs = std::fs::read_to_string(dir.join("openssl-libs")).expect("openssl-libs in the native cache");
        Native {
            lt_include: dir.join("libtorrent/include"),
            lt_lib_dir: dir.join("libtorrent/lib"),
            ssl_include: dir.join("openssl/include"),
            ssl_lib_dir: dir.join("openssl/lib"),
            ssl_libs: libs.lines().map(str::to_string).collect(),
        }
    }

    /// Copies into a folder beside `dir` and renames it into place, so a build
    /// that dies halfway, or another one racing this, never leaves a half
    /// entry that looks complete.
    fn store(&self, dir: &Path) -> std::io::Result<()> {
        let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
        let tmp = dir.with_extension(format!("tmp{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy_dir(&self.lt_include, &tmp.join("libtorrent/include"))?;
        std::fs::create_dir_all(tmp.join("libtorrent/lib"))?;
        std::fs::copy(self.lt_lib_dir.join(lt_archive(msvc)), tmp.join("libtorrent/lib").join(lt_archive(msvc)))?;
        copy_dir(&self.ssl_include, &tmp.join("openssl/include"))?;
        copy_dir(&self.ssl_lib_dir, &tmp.join("openssl/lib"))?;
        std::fs::write(tmp.join("openssl-libs"), self.ssl_libs.join("\n"))?;
        std::fs::write(tmp.join("ready"), "")?;
        if std::fs::rename(&tmp, dir).is_err() {
            // Another build got there first with the same inputs.
            std::fs::remove_dir_all(&tmp)?;
        }
        Ok(())
    }

    fn includes(&self, boost: &Path) -> [PathBuf; 3] {
        [self.lt_include.clone(), boost.to_path_buf(), self.ssl_include.clone()]
    }

    fn link(&self) {
        println!("cargo:rustc-link-search=native={}", self.lt_lib_dir.display());
        println!("cargo:rustc-link-lib=static=torrent-rasterbar");
        println!("cargo:rustc-link-search=native={}", self.ssl_lib_dir.display());
        for lib in &self.ssl_libs {
            println!("cargo:rustc-link-lib=static={lib}");
        }
        println!("cargo:include={}", self.ssl_include.display());
        println!("cargo:lib={}", self.ssl_lib_dir.display());
    }
}

/// The cache entry for this build, or None when ZTORRENT_NATIVE_CACHE is not
/// set. The key covers what decides the compiled output: this script, the
/// pinned libtorrent and OpenSSL, Boost, the target, the profile, the
/// compiler and the flags it is handed.
fn native_cache(boost: &Path) -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("ZTORRENT_NATIVE_CACHE")?);
    let mut key = Sha256::new();
    key.update(include_bytes!("build.rs"));
    for part in [LT_VERSION, LT_SHA256, openssl_src::version()] {
        key.update(part);
        key.update([0]);
    }
    for var in ["TARGET", "PROFILE", "OPT_LEVEL", "DEBUG", "CARGO_CFG_TARGET_FEATURE", "MACOSX_DEPLOYMENT_TARGET", "LIBTORRENT_SRC", "CC", "CXX", "CFLAGS", "CXXFLAGS", "AR"] {
        key.update(std::env::var(var).unwrap_or_default());
        key.update([0]);
    }
    key.update(std::fs::read(boost.join("boost/version.hpp")).unwrap_or_default());
    let compiler = cc::Build::new().cpp(true).get_compiler();
    key.update(compiler.path().as_os_str().as_encoded_bytes());
    if !compiler.is_like_msvc() {
        // MSVC's path names its version; clang and gcc have to be asked.
        if let Ok(version) = Command::new(compiler.path()).arg("--version").output() {
            key.update(&version.stdout);
        }
    }
    let hex: String = key.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
    Some(root.join(format!("{}-{hex}", std::env::var("TARGET").unwrap())))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn configure(build: &mut cc::Build, msvc: bool) {
    if msvc {
        build.flag("/bigobj").flag("/permissive-").flag("/utf-8").flag("/Zc:__cplusplus").flag("/EHsc");
    } else {
        build.flag("-fexceptions").flag_if_supported("-Wno-deprecated-declarations");
    }
}

/// The `.cpp` entries of one `set(<name> ...)` list in libtorrent's CMakeLists,
/// so the file set follows the pinned release instead of a copy kept here.
fn cmake_list(cmake: &str, name: &str) -> Vec<String> {
    let head = format!("set({name}");
    let start = cmake
        .match_indices(&head)
        .map(|(i, _)| i + head.len())
        .find(|&i| cmake[i..].starts_with(char::is_whitespace))
        .unwrap_or_else(|| panic!("no set({name} ...) in libtorrent's CMakeLists.txt"));
    let body = &cmake[start..start + cmake[start..].find(')').expect("unterminated set()")];
    let files: Vec<String> = body
        .lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .flat_map(str::split_whitespace)
        .filter(|word| word.ends_with(".cpp"))
        .map(str::to_string)
        .collect();
    assert!(!files.is_empty(), "set({name} ...) lists no sources");
    files
}

fn libtorrent_source(out: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("LIBTORRENT_SRC") {
        return PathBuf::from(dir);
    }
    let dir = out.join(format!("libtorrent-rasterbar-{LT_VERSION}"));
    if dir.join("CMakeLists.txt").is_file() {
        return dir;
    }
    let name = format!("libtorrent-rasterbar-{LT_VERSION}.tar.gz");
    let tarball = out.join(&name);
    if sha256(&tarball).as_deref() != Some(LT_SHA256) {
        let url = format!("https://github.com/arvidn/libtorrent/releases/download/v{LT_VERSION}/{name}");
        let got = Command::new("curl").args(["-sSfL", "--retry", "3", "-o"]).arg(&tarball).arg(&url).status();
        assert!(
            got.is_ok_and(|s| s.success()),
            "could not download {url}; set LIBTORRENT_SRC to an unpacked libtorrent {LT_VERSION} source tree"
        );
        let sum = sha256(&tarball);
        assert!(sum.as_deref() == Some(LT_SHA256), "{name} has SHA-256 {sum:?}, expected {LT_SHA256}");
    }
    // On Windows, the system's bsdtar: Git's GNU tar, often first on PATH there,
    // reads the drive letter in `C:\...` as a remote host.
    let system_tar = std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join("System32").join("tar.exe"));
    let tar = system_tar.filter(|p| cfg!(windows) && p.is_file()).unwrap_or_else(|| PathBuf::from("tar"));
    let unpacked = Command::new(tar).arg("-xzf").arg(&tarball).arg("-C").arg(out).status();
    assert!(unpacked.is_ok_and(|s| s.success()), "could not unpack {}", tarball.display());
    dir
}

fn sha256(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect())
}

fn boost_include() -> PathBuf {
    let explicit = std::env::var_os("BOOST_INCLUDE").map(PathBuf::from).or_else(|| std::env::var_os("BOOST_ROOT").map(PathBuf::from));
    let candidates = explicit.into_iter().chain(["/opt/homebrew/include", "/usr/local/include", "/usr/include"].map(PathBuf::from));
    for dir in candidates {
        if dir.join("boost/asio.hpp").is_file() {
            return dir;
        }
    }
    panic!("Boost headers not found: set BOOST_INCLUDE to the directory that contains boost/ (macOS: brew install boost; Debian/Ubuntu: apt install libboost-dev)");
}
