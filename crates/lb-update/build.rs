//! Records the Rust target triple the crate is compiled for: the updater asks for
//! the release asset built for exactly this target (`librebooks-<tag>-<target>.*`).

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=LB_UPDATE_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
