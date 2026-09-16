use std::env;
use std::path::PathBuf;

use xbuild::{CLibrary, Result, bail, bail_unsupported};

fn main() -> Result<()> {
    // Reuse xbuild's existing binary-type linker selection for this experiment.
    let binary_type = if cfg!(feature = "ironwood_target_native_compile_only") {
        "firmware_ironwood"
    } else {
        "firmware"
    };
    xbuild::build_and_link(binary_type, |lib| {
        lib.import_lib("io")?;
        lib.import_lib("upymod")?;

        lib.add_includes(["."]);

        if cfg!(feature = "ironwood_target_native_compile_only") {
            if xbuild::current_model_id()? != "T3T1"
                || !cfg!(feature = "mcu_stm32u58")
                || cfg!(feature = "secmon_layout")
                || cfg!(feature = "production")
            {
                bail!("synthetic native integration requires nonproduction T3T1 without secmon layout");
            }
            lib.add_define("IRONWOOD_TARGET_NATIVE_COMPILE_ONLY", Some("1"));
        }

        lib.add_include("../../rust"); // Cyclic dependency

        lib.add_sources(["main.c", "header.S", "boot_image_embdata.c"]);

        if cfg!(feature = "mcu_stm32") {
            lib.add_source("stm32/coreapp_header.S");
        } else {
            bail_unsupported!()
        }

        if cfg!(feature = "app_loading") {
            lib.add_source("../../api/trezor_api_v1_impl.c");
        }

        if cfg!(feature = "force_bootloader_upgrade") {
            lib.add_define("FORCE_BOOTLOADER_UPGRADE", Some("1"));
        }

        lib.embed_binary(
            xbuild::vendor_header_path("../../models", "firmware")?,
            "vendorheader",
        )?;

        embed_bootloader_binary(lib)?;
        embed_kernel_binary(lib)?;

        if cfg!(feature = "nrf") {
            embed_nrf_app_binary(lib)?;
        }

        Ok(())
    })
}

fn embed_kernel_binary(lib: &mut CLibrary) -> Result<()> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let kernel = out_dir.join("../../../kernel.bin");
    lib.embed_binary(&kernel, "kernel")
}

fn embed_bootloader_binary(lib: &mut CLibrary) -> Result<()> {
    let model_id = xbuild::current_model_id()?;
    let model_dir = format!("../../models/{}", model_id);
    let suffix = if cfg!(feature = "bootloader_devel") {
        "_devel"
    } else {
        ""
    };

    let bootloader = format!("{model_dir}/bootloaders/bootloader_{model_id}{suffix}.bin");

    if cfg!(feature = "boot_ucb") {
        // embed uncompressed bootloader image
        lib.embed_binary(bootloader, "bootloader")?;
    } else {
        lib.embed_compressed_binary(bootloader, "bootloader")?;
    }

    Ok(())
}

fn embed_nrf_app_binary(lib: &mut CLibrary) -> Result<()> {
    let model_id = xbuild::current_model_id()?;
    let model_dir = format!("../../models/{}", model_id);
    let suffix = if cfg!(feature = "bootloader_devel") {
        "-dev"
    } else {
        ""
    };
    let nrf_app = format!("{model_dir}/trezor-ble{suffix}.bin");
    lib.embed_binary(&nrf_app, "nrf_app")
}
