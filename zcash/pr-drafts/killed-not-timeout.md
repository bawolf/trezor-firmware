# fix(core): report a stopped extapp instead of a timeout

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches.
Reproduction status: standalone repro on the emulator with a scratch fault in
the Tron sample; no committed test.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/killed-not-timeout @ `fc136e637c`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

An app that faults while handling a request sends nothing, so `run.py`
reported the crash as "Timeout waiting for message", the same as a slow app.

**Reproduce.** Make the Tron sample panic on a sentinel path (scratch change,
not for commit), at the top of `get_address` in `sdk/apps/tron/src/get_address.rs`:

```rust
if msg.address_n.first() == Some(&0xDEAD_BEEF) {
    panic!("probe: intentional fault");
}
```

Build it with `xtask modular build -p tron -m t3w1 --lang en -d -e`, and from
`sdk/apps/tron` run a test that calls
`tron_ext.get_address(session, instance_id, [0xDEADBEEF], show_display=False)`
against a T3W1 `--apps` emulator. Observed:

```
before: DataError: Timeout waiting for message
after:  DataError: Task stopped: 224053394
```

**Cause.** The timeout branch does not check `image.is_running()`, although
the top of the loop does.

**Fix.** On timeout, an image that is no longer running fails with the loop's
existing `DataError("Task stopped: …")`. A slow app that is still running gets
the timeout as before.

**Tests.** The repro above. A permanent test belongs with the applet fixtures
of `bieleluk/extapp-device-tests`, which are not on sdk-wip.

### Notes for QA
An app that faults now reports "Task stopped" instead of "Timeout waiting for
message".
