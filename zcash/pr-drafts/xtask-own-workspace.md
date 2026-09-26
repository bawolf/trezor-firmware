# xtask: build an app that is its own workspace (reserve, not proposed)

Status: **not proposed.** Kept in reserve; nothing in our series needs it any
more, since the app now lives in the `sdk/apps` workspace. If it is ever sent,
it is a note for the owner of `bieleluk/sdk-wip` (#7516) or `cepetr/apptool`,
which is reworking this tool, not a PR.
Commit 1 is independent and could go alone as a one-line fix; it is in our
integrated series as `5c203f7a42`.
Reproduction status: none needed for commit 1 (an error message); commit 2,
modular-xtask tests only.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/xtask-own-workspace @ `e3593e829c`
(two commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

- `8f6a34cd6b` fix(xtask): name translation-style in its error message
- `e3593e829c` feat(xtask): build an app that is its own workspace

`translations.rs` said "running py-style" when `translation-style` was run in
a workspace without a project name. Commit 1 names the right command.

Commit 2 lets an app whose dependencies cannot share the `sdk/apps` lockfile
live in `sdk/apps/<app>` as a workspace of its own: `xtask modular <cmd> -p
<app>` runs in that directory when it holds a workspace root. Device tests,
py-style and translation-style use the package's manifest directory, and proof
generation finds `extapp_tool.py` in the nearest ancestor that has it.

Limitation: such an app publishes its own artifacts directory, so its dev root
packet covers only that app and it cannot be installed next to the `sdk/apps`
apps without merging the directories.

**Tests.** modular-xtask tests pass.
