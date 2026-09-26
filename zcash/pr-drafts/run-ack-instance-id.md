# fix(core): check the instance id of the host's answers to an extapp

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches. Latent while one app instance runs at a
time.
Reproduction status: observed only with a device test of an out-of-tree app
(fails before, passes after); no test on this branch.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/run-ack-instance-id @ `1d0efa2f08`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`run.py` checks the instance id of the host request that starts an app's run,
but forwarded the host's answers to the app's `WireContinue` and `WireError` to
the running app whatever instance they named.

**Reproduce.** Start a request that makes the app ask the host for more data,
then answer with an `ExtAppMessage` whose `instance_id` is another app's:

```
before: the app takes the answer (the test's pytest.raises: DID NOT RAISE TrezorFailure)
after:  Failure: DataError: Invalid instance ID: …
```

**Cause.** Both `ack = await context.call(response, ExtAppMessage)` sites check
`ack.message_id` but not `ack.instance_id`.

**Fix.** A mismatched instance id stops the app and fails the request, as an
invalid message id does.

**Tests.** None on this branch: exercising it needs an app that asks the host
for data mid-request; the Ethereum sample's `SignTx` with data would do. Core
unit tests pass.

### Notes for QA
A host that answers with the wrong instance id gets `DataError`. Normal hosts
are unaffected.
