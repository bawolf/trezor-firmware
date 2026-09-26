use xbuild::{CLibrary, Result, bail_unsupported};

pub fn def_module(lib: &mut CLibrary) -> Result<()> {
    lib.add_include("rng/inc");
    lib.add_rust_bindings(add_rust_bindings)?;

    if cfg!(feature = "emulator") {
        lib.add_define("USE_INSECURE_PRNG", Some("1"));

        lib.add_sources([
            "rng/unix/rng.c",
            "rng/unix/rng_use_flags.c",
            "rng/unix/rng_mock.c",
        ]);
    } else if cfg!(feature = "mcu_stm32") {
        lib.add_sources(["rng/stm32/rng.c", "rng/stm32/rng_use_flags.c"]);
    } else {
        bail_unsupported!();
    }

    Ok(())
}

fn add_rust_bindings(builder: bindgen::Builder) -> Result<bindgen::Builder> {
    let builder = builder
        .header("rng/inc/sys/rng.h")
        .allowlist_function("rng_fill_buffer");
    Ok(builder)
}
