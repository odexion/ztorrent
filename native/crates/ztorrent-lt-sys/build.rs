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

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

const LT_VERSION: &str = "2.1.1";
const LT_SHA256: &str = "0f163516ecef2e3331500266751de3098835a3c3ae0c2290448046c632bc0e93";

fn main() {
    for var in ["BOOST_INCLUDE", "BOOST_ROOT", "LIBTORRENT_SRC", "ZTORRENT_SKIP_NATIVE", "MACOSX_DEPLOYMENT_TARGET"] {
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

    let lt = libtorrent_source(&out);
    let boost = boost_include();
    let ssl = openssl_src::Build::new()
        // Where a static OpenSSL looks for trusted certificates at run time.
        // Windows needs none: libtorrent loads the system ROOT store itself. On
        // Linux the app also points SSL_CERT_FILE at the distribution's bundle.
        .openssl_dir(if os == "linux" { "/usr/lib/ssl" } else { "/etc/ssl" })
        .build();

    // Definitions the headers must see identically in libtorrent and in the
    // bridge, or the two disagree about the layout of the same types.
    let mut public: Vec<(&str, Option<&str>)> = vec![
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
    let includes = [lt.join("include"), boost.clone(), ssl.include_dir().to_path_buf()];

    // The bridge first: with static archives the linker wants each library
    // before the ones it depends on.
    let mut bridge = cxx_build::bridge("src/lib.rs");
    bridge.file("src/bridge.cpp").std("c++17").includes(&includes);
    for (key, value) in &public {
        bridge.define(key, *value);
    }
    configure(&mut bridge, msvc);
    bridge.compile("ztorrent-lt-bridge");

    let cmake = std::fs::read_to_string(lt.join("CMakeLists.txt")).expect("libtorrent CMakeLists.txt");
    let mut lib = cc::Build::new();
    lib.cpp(true).std("c++17").includes(&includes).include(lt.join("deps/try_signal"));
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
    for (key, value) in &public {
        lib.define(key, *value);
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
    lib.compile("torrent-rasterbar");

    ssl.print_cargo_metadata();
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
