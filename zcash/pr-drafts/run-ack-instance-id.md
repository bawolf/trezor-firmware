# fix(core): check the instance id of the host's answers to an extapp

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches. Latent while one app instance runs at a
time. No committed test; a scratch test with the Ethereum sample reproduces it.
Rerun 2026-09-26 on sdk-wip and on the pushed tip `1d0efa2f08` (T3W1 non-frozen
`--apps` emulator built at each): receipts
`pr-drafts-rerun/logs/ack-instance-{base,tip}.log`. Local macOS build fix only
(`15de3664a6` cherry-picked for the emulator app); not needed on Linux.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/run-ack-instance-id @ `1d0efa2f08`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`run.py` checks the instance id of the host request that starts an app's run,
but forwarded the host's answers to the app's `WireContinue` and `WireError`
to the running app whatever instance they named. A mismatched instance id now
stops the app and fails the request, as an invalid message id does.

Tested on a T3W1 `--apps` emulator with the Ethereum sample
(`xtask modular build -p ethereum -m t3w1 --lang en -d -e`) and the scratch
test [repro/run-ack-instance-id/test_zz_ack_instance_id.py](repro/run-ack-instance-id/test_zz_ack_instance_id.py)
in `sdk/apps/ethereum/tests/`. It starts a `SignTx` with 3000 bytes of data
and answers the app's `TxRequest` under instance id + 1:

```
before: the app took the answer and asks for 952 more bytes
after:  DataError: Invalid instance ID: 224053395
```
