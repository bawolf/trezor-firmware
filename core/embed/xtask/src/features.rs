use std::io::IsTerminal;
use std::{env, io, process};

use anyhow::{Result, bail};

use crate::options::ResolvedBuildArgs;
use crate::{config, helpers};

#[derive(Debug)]
pub struct ResolvedBuildFeatures {
    pub features: Vec<String>,
    pub target_triple: Option<&'static str>,
    pub board_header: String,
}

/// Resolves cargo features and target triple from the provided build
/// arguments.
///
/// Option-dependent features come from the `[build-options]` table of the
/// project's project.toml; board- and model-intrinsic features come from the
/// model/board TOML configs filtered by the project's `uses` list. Only
/// features tied to build mechanics (model selection, emulator, asan) are
/// added directly here.
pub fn resolve_features(args: &ResolvedBuildArgs) -> Result<ResolvedBuildFeatures> {
    if args.production {
        if args.storage_insecure_testing_mode {
            bail!("storage_insecure_testing_mode cannot be used in production builds");
        }
        if args.disable_optiga {
            bail!("disable_optiga cannot be used in production builds");
        }
        if args.disable_tropic {
            bail!("disable_tropic cannot be used in production builds");
        }
    }

    let mut features: Vec<String> = vec![args.model.feature_name()];

    if args.emulator {
        features.push("emulator".into());
    }

    let project_config = config::ProjectConfig::load(args.project)?;

    for activated in project_config.options.resolve(args) {
        features.push(activated.feature);
    }

    let model_config = args.model.config()?;

    let board_id = args
        .board
        .clone()
        .unwrap_or_else(|| model_config.default_board.clone());

    let board_def = config::resolve_board_definition(
        &model_config,
        &board_id,
        &project_config,
        args.project,
        args.emulator,
    )?;

    // Remove features that are disabled by command-line flags.
    let mut board_features = board_def.features;
    if args.disable_optiga {
        board_features.retain(|f| f != "optiga");
    }
    if args.disable_tropic {
        board_features.retain(|f| f != "tropic");
    }
    features.extend(board_features);

    let target_triple = if args.emulator {
        None
    } else {
        Some(model_config.target_triple()?)
    };

    Ok(ResolvedBuildFeatures {
        features,
        target_triple,
        board_header: board_def.board_header,
    })
}

/// Resolves `CARGO_TERM_COLOR` to `always`/`never` for the spawned Cargo.
///
/// Build scripts see only pipes (Cargo captures their output), so `xtask` -
/// the last process attached to the real terminal - decides for them; `xbuild`
/// reads the result to color C compiler diagnostics. An explicit
/// `always`/`never` in the environment is left alone (the child inherits it).
fn forward_color_choice(cmd: &mut process::Command) {
    let explicit = env::var("CARGO_TERM_COLOR").is_ok_and(|v| v == "always" || v == "never");
    if explicit {
        return;
    }

    // https://no-color.org
    let color = if env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        "never"
    // https://bixense.com/clicolors
    } else if io::stderr().is_terminal()
        || env::var_os("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0")
    {
        "always"
    } else {
        "never"
    };

    cmd.env("CARGO_TERM_COLOR", color);
}

/// Configures a cargo command with the appropriate arguments and features.
pub fn configure_cargo(args: &ResolvedBuildArgs, cmd: &mut process::Command) -> Result<()> {
    let resolved = resolve_features(args)?;
    let mut rebuild_std = false;

    cmd.args(["--package", args.project.package_name()]);
    cmd.args(["--features", &resolved.features.join(",")]);
    cmd.args(["--profile", args.cargo_profile_name()]);
    cmd.env("TREZOR_BOARD_HEADER", &resolved.board_header);
    cmd.env("SCM_REVISION", helpers::git_revision()?);

    if args.cargo_profile_name() == "release" {
        // Required by panic-immediate-abort in the release profile
        rebuild_std = true;
    }

    if let Some(triple) = resolved.target_triple {
        cmd.args(["--target", triple]);
    }

    if args.emit_memory_analysis {
        // See https://nnethercote.github.io/perf-book/type-sizes.html#measuring-type-sizes for more details
        // Also adds an ELF section with Rust functions' stack sizes. See:
        // - https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/emit-stack-sizes.html
        // - https://blog.japaric.io/stack-analysis/
        // - https://github.com/japaric/stack-sizes/
        //
        // Use --config instead of RUSTFLAGS env so that rustflags in .cargo/config.toml
        // are not overridden (RUSTFLAGS env has higher precedence and replaces
        // them entirely).
        cmd.args([
            "--config",
            "build.rustflags=[\"-Zprint-type-sizes\", \"-Zemit-stack-sizes\"]",
        ]);
    }

    if args.emulator && args.asan {
        // -Zsanitizer=address is a rustc flag passed via RUSTFLAGS.
        //
        // Without an explicit --target, cargo compiles proc-macros and the firmware in
        // the same pass and RUSTFLAGS leaks into proc-macro crates, causing
        // "can't find crate" errors. Passing --target explicitly (even the same
        // triple as the host) makes cargo separate the host (proc-macros /
        // build scripts) and target (firmware) compilation units, so RUSTFLAGS
        // only reaches the firmware crates.
        cmd.args(["--target", &helpers::host_triple()?]);
        cmd.args([
            "--config",
            "build.rustflags=[\"-Zsanitizer=address\", \"-Clink-arg=-lgcc_s\"]",
        ]);

        // Rebuild standard library to be compiled with sanitizer instrumentation
        rebuild_std = true;
    }

    if args.timings {
        cmd.arg("--timings");
    }

    if args.xbuild_trace {
        // Cargo does not pass its own verbosity on to build scripts, so
        // `xbuild` reads the request from the environment instead.
        cmd.env("XBUILD_TRACE", "1");
        // `-vv`, not `-v`: Cargo relays build script output only at the second
        // verbosity level, and would otherwise discard what `xbuild` logs.
        cmd.arg("-vv");
    } else if args.verbose {
        cmd.arg("--verbose");
    }

    if rebuild_std {
        if args.ironwood && matches!(args.project, crate::args::Project::Firmware) {
            cmd.arg("-Zbuild-std=core,alloc");
        } else {
            cmd.arg("-Zbuild-std=core");
        }
    }

    forward_color_choice(cmd);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{BuildArgs, Model, Project};
    use crate::options::BuildOptions;

    fn ironwood_build_args(project: Project, model: Model) -> BuildArgs {
        BuildArgs {
            project,
            model,
            emulator: false,
            preset: None,
            options: BuildOptions {
                ironwood: Some(true),
                ..BuildOptions::default()
            },
        }
    }

    #[test]
    fn omits_ironwood_by_default() {
        let args = ResolvedBuildArgs {
            model: Model::T3T1,
            frozen: true,
            pyopt: true,
            ..ResolvedBuildArgs::default()
        };

        let features = resolve_features(&args).unwrap().features;
        assert!(!features.contains(&"ironwood".to_string()));
    }

    #[test]
    fn enables_ironwood_for_safe_5_firmware() {
        let args = ResolvedBuildArgs {
            model: Model::T3T1,
            ironwood: true,
            frozen: true,
            pyopt: true,
            ..ResolvedBuildArgs::default()
        };

        let features = resolve_features(&args).unwrap().features;
        assert!(features.contains(&"ironwood".to_string()));
    }

    #[test]
    fn enables_ironwood_for_safe_5_firmware_emulator() {
        let args = ResolvedBuildArgs {
            model: Model::T3T1,
            emulator: true,
            ironwood: true,
            frozen: true,
            pyopt: true,
            ..ResolvedBuildArgs::default()
        };

        let features = resolve_features(&args).unwrap().features;
        assert!(features.contains(&"ironwood".to_string()));
    }

    #[test]
    fn rejects_ironwood_for_other_models() {
        let error = ResolvedBuildArgs::from_build_args(&ironwood_build_args(
            Project::Firmware,
            Model::T3W1,
        ))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "--ironwood is supported only for Safe 5/T3T1 firmware builds"
        );
    }

    #[test]
    fn rejects_ironwood_for_other_projects() {
        let error =
            ResolvedBuildArgs::from_build_args(&ironwood_build_args(Project::Kernel, Model::T3T1))
                .unwrap_err();
        assert_eq!(
            error.to_string(),
            "--ironwood is supported only for Safe 5/T3T1 firmware builds"
        );
    }

    #[test]
    fn rejects_ironwood_for_bitcoin_only_firmware() {
        let mut args = ironwood_build_args(Project::Firmware, Model::T3T1);
        args.options.btc_only = Some(true);
        let error = ResolvedBuildArgs::from_build_args(&args).unwrap_err();
        assert_eq!(
            error.to_string(),
            "--ironwood cannot be combined with --btc-only"
        );
    }

    #[test]
    fn omits_ironwood_from_firmware_dependency_builds() {
        let firmware_args = ResolvedBuildArgs::from_build_args(&ironwood_build_args(
            Project::Firmware,
            Model::T3T1,
        ))
        .unwrap();
        let firmware_features = resolve_features(&firmware_args).unwrap().features;
        assert!(firmware_features.contains(&"ironwood".to_string()));

        let dependency_args = ResolvedBuildArgs {
            project: Project::Kernel,
            ..firmware_args
        };

        let features = resolve_features(&dependency_args).unwrap().features;
        assert!(!features.contains(&"ironwood".to_string()));
    }

    #[test]
    fn accepts_explicitly_disabled_ironwood_for_unsupported_targets() {
        let args = BuildArgs {
            project: Project::Kernel,
            model: Model::T3W1,
            emulator: false,
            preset: None,
            options: BuildOptions {
                ironwood: Some(false),
                ..BuildOptions::default()
            },
        };
        let resolved = ResolvedBuildArgs::from_build_args(&args).unwrap();

        let features = resolve_features(&resolved).unwrap().features;
        assert!(!features.contains(&"ironwood".to_string()));
    }

    #[test]
    fn rejects_insecure_storage_in_production_builds() {
        let args = ResolvedBuildArgs {
            production: true,
            storage_insecure_testing_mode: true,
            ..ResolvedBuildArgs::default()
        };

        let error = resolve_features(&args).unwrap_err();
        assert!(error.to_string().contains("production"));
    }

    #[test]
    fn rejects_disable_optiga_in_production_builds() {
        let args = ResolvedBuildArgs {
            production: true,
            disable_optiga: true,
            ..ResolvedBuildArgs::default()
        };

        let error = resolve_features(&args).unwrap_err();
        assert!(error.to_string().contains("production"));
    }

    #[test]
    fn rejects_disable_tropic_in_production_builds() {
        let args = ResolvedBuildArgs {
            production: true,
            disable_tropic: true,
            ..ResolvedBuildArgs::default()
        };

        let error = resolve_features(&args).unwrap_err();
        assert!(error.to_string().contains("production"));
    }

    #[test]
    fn ignores_options_the_project_does_not_map() {
        // prodtest doesn't map `disable-animation` (the package has no such
        // feature), so the option is ignored like any other unmapped option.
        let args = ResolvedBuildArgs {
            project: Project::Prodtest,
            frozen: true,
            pyopt: true,
            disable_animation: true,
            ..ResolvedBuildArgs::default()
        };

        let features = resolve_features(&args).unwrap().features;
        assert!(!features.contains(&"disable_animation".to_string()));
    }
}
