//! Build script: macOS only.
//!
//! The `screencapturekit` crate links a small Swift bridge and adds the Swift runtime
//! search path of a full Xcode install. With only the Command Line Tools installed that
//! path does not exist, and linking fails with
//! `Undefined symbols … __swift_FORCE_LOAD_$_swiftCompatibility56`.
//! Here we ask `xcrun` where the active `swift` lives and add its
//! `lib/swift/macosx` folder, which works for both Xcode and the Command Line Tools.
//! It also sets the runtime search path for the Swift libraries that macOS provides.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // The Swift bridge loads Swift runtime libraries (e.g. libswift_Concurrency.dylib)
    // via @rpath. macOS ships them in /usr/lib/swift. The screencapturekit crate emits
    // this rpath too, but Cargo applies a dependency's `rustc-link-arg` only to that
    // dependency, never to our binary, so it has to be set here.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    let swift = Command::new("xcrun")
        .args(["--find", "swift"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));

    let Some(swift) = swift else {
        println!("cargo:warning=could not locate swift via xcrun; install the Xcode Command Line Tools (xcode-select --install)");
        return;
    };

    // <toolchain>/usr/bin/swift  ->  <toolchain>/usr/lib/swift/macosx
    match swift.parent().and_then(|bin| bin.parent()) {
        Some(usr) if usr.join("lib/swift/macosx").is_dir() => {
            println!(
                "cargo:rustc-link-search=native={}",
                usr.join("lib/swift/macosx").display()
            );
        }
        _ => println!(
            "cargo:warning=no Swift runtime libraries found next to {}",
            swift.display()
        ),
    }
}
