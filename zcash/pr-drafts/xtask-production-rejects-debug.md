# fix(xtask): reject --debug in production builds

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR.
`modular-xtask` exists only on the draft SDK branches; `cepetr/apptool` is
reworking it into `xtask apps` (no such check there either), so this may
belong there.
Rerun 2026-09-26 on sdk-wip and on the pushed tip `ef50fa6a04`: the build
command (`pr-drafts-rerun/logs/xtask-prod-debug-{base,tip}-cli.log`) and
modular-xtask's tests (`xtask-prod-debug-tip-test.log`). No local changes.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/xtask-production-rejects-debug @ `ef50fa6a04`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`--debug` enables the app's and the SDK's `debug` feature (logging and
debug-only handlers) and the debug-fw profile, and nothing stopped it from
being combined with `--production`. `resolve_features` now refuses the
combination, as Core's xtask refuses insecure options in production builds.

`xtask modular build -p tron -m t3w1 --lang en --production --debug` from
`core/embed`: on sdk-wip it builds with `--features model_t3w1,lang_en,log_level_info,debug --profile debug-fw`;
with this change it stops with `Error: --debug cannot be used in production builds`.
New test `resolve_features_rejects_debug_in_production_builds`; the
modular-xtask tests pass (36 + 5 doctests).
