//! The crate declares `links = "ironwood"` like every other crate under
//! `core/embed/`, which requires a build script. It has no C sources and no
//! generated bindings, so there is nothing else to do here.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
}
