# fix(xtask): reject --debug in production builds

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR.
`modular-xtask` exists only on the draft SDK branches; `cepetr/apptool` is
reworking it into `xtask apps`, so this may belong there.
Reproduction status: unit test added; not run against the base.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/xtask-production-rejects-debug @ `ef50fa6a04`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`--debug` enables the app's and the SDK's `debug` feature (logging and
debug-only handlers) and the debug-fw profile, and nothing stopped it from
being combined with `--production`. `resolve_features` now refuses the
combination, as Core's xtask refuses insecure options in production builds.

**Reproduce.** `xtask modular build -p tron -m t3w1 --production --debug`
resolves its features without complaint on sdk-wip (code reading of
`BuildArgs::resolve_features`).

**Fix.** `ensure!(!(self.production && self.debug), "--debug cannot be used in production builds")`
at the top of `resolve_features`.

**Tests.** `resolve_features_rejects_debug_in_production_builds`; modular-xtask
tests pass (41/41 on the branch).

### Notes for QA
`--production --debug` fails with "--debug cannot be used in production
builds". Every other combination is unchanged.
