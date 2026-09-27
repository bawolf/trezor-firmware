# Introduction to the platform team (issue or email)

Status: draft, not sent. Send as an issue on trezor/trezor-firmware or as an
email to jan.setina@satoshilabs.com and jan.matejek@satoshilabs.com, the two
contacts in Hannsek's reply on #6770 (a Kaspa proposal, closed not planned).
Fill in the video link before sending. Device runs so far:
- On 2026-09-27, the series plus the demo-only commits (`7a438a4399`, bundle `extapp-7a438a4399`) ran on
  the Safe 7 under Trezor Suite, from the private Suite branch: add account, receive, and a
  testnet send (txid `689147f2…`, block 4,401,482).
- An earlier build ran with trezorlib on 2026-09-26.

The Suite side is T-RSL, so it is offered privately, not linked publicly.

---

**Title:** Shielded Zcash as an extapp, and fixes found on the WIP SDK

Hi,

We built a shielded Zcash app on the extapp SDK from `bieleluk/sdk-wip`
(#7516). It shows Orchard unified addresses, exports a viewing key and signs
PCZTs that spend from the Ironwood pool. It runs on T3W1 and T3T1 emulators and
on a T3W1 development device, where we also drove it from Trezor Suite: we built the
Suite side on the ExtApp loading in #32700, in a private checkout. A shielded testnet
send, signed on the device, was mined in block 4,401,482. Video: <link>

#6962 puts Zcash after the modularization, and Hannsek's reply on #6770
offers the modularization alpha to third-party contributors. So we are asking
before opening anything: how would you like contributions to the WIP SDK?

Everything is on our fork, based on `bieleluk/sdk-wip` @ `4cd93ff4d8`:
- 15 small branches (`extapp/*`): 14 with fixes and features for `run.py`,
  the Rust bridges, `trezor-app-sdk` and `modular-xtask`, and one fix to
  Core's Zcash address decoder on `main`. Most are a few lines with a test or
  a reproduction.
- A proposed Core service that derives ZIP-32 Orchard account keys for an app
  that declares them, after a hold-to-confirm screen. The app gets the account's
  spending key. That is a design decision for you, and we would like your view
  before going further.
- The app itself (`sdk/apps/zcash`), which depends on all of the above.

The integrated series: https://github.com/bawolf/trezor-firmware/tree/zcash/extapp-series

Questions:
1. For fixes to code that exists only on the SDK draft branches, do you prefer
   patches or branches sent to the branch owners, PRs against
   `bieleluk/sdk-wip`, or should we wait until the code reaches `main`?
2. Would you consider a Core service that hands an app a spending key? If
   not, what would you prefer?
3. Is a Zcash app something you would review in this form at all, or should it
   stay out of tree until the SDK is released?

The Suite changes (Connect methods over ExtApp, and a desktop shielded account)
are in a private repository, because of the Suite licence. We can give you access
if that is useful.

We can split, rebase or rework any of it. Nothing is urgent.

Thanks,
Bryant Wolf
