# Shielded Zcash as a Trezor app (extapp): plan

Date: 2026-09-25. **Status:** plan. The user decided on 2026-09-25 to build shielded Zcash
toward Trezor's app platform:
- Safe 7 first, since it has an app arena on `main`.
- Then the Safe 5, on Trezor's alpha arena commit.
- Then show Trezor a working branch and a video.

Research: `APP-LOADING.md` in the author's `.context/product-scaffold/flash/`, summarised below.

## Why

A loaded app runs from RAM and costs no flash for Trezor users who do not install it. The
built-in series costs ~106 KB of Safe 5 flash, plus 8–9 KB even with the feature off. That
feature-off cost is a bug in the series, now moot for the app. Trezor says shielded ZEC comes
"after the firmware modularization" (#6962). The platform is alpha and open to third-party
contributors (#6770). This puts the work in the form Trezor has said it will ship.

## What exists upstream (checked 2026-09-25)

- **Merged, off by default** (`--apps`):
  - the app arena (T3W1: 384 KiB), ML-DSA-44 root-packet verification, applet privileges and
    downgrade protection;
  - the emulator loads apps with `dlopen` and dev root keys.
- **Open, or on WIP branches** (`bieleluk/*`):
  - extapp orchestration (#7947, `run.py` a stub);
  - the IPC update (#7957);
  - the Rust SDK (`extapp-rust-sdk`);
  - device tests (`extapp-device-tests`);
  - Ethereum/Tron apps, the coreapp UI and crypto services, and the Safe 5 arena
    (`modular-ethereum-wip`, 231 KiB, one framebuffer).
- **Rules:**
  - Apps are Rust, unprivileged, and show only predefined screens.
  - Seed-derived keys come only through coreapp's Crypto service, which on the WIP branch is
    BIP-32/SLIP-10, one curve per app.
  - Trezor signs admission to every ring.

## The one real gap: Zcash keys for an app

Orchard keys are ZIP-32, derived from the raw seed with BLAKE2b, and the app never sees the
seed.

**Plan: add one coreapp Crypto-service operation on our branch.** It derives, for account `a`
on the app's registered path prefix (`m/32'/133'/a'` or `m/32'/1'/a'`):
- the ZIP-32 Orchard spending key;
- the ZIP-32 seed fingerprint;
- the backup-strength bit the ZIP-315 warning needs.

It is BLAKE2b only (already in trezor-crypto), so coreapp gains no Pasta code.

**Behind the new "direct key access" entitlement.** The app receives `sk` and does all Orchard
and RedPallas work itself. Signing inside coreapp would put Pasta back into firmware flash and
defeat the purpose.

**Transparent inputs** (phase 2) would later need secp256k1 on `m/44'/133'/…` as well. That is
a multi-curve allowance, deferred.

**Trezor decides.** This is the proposal we bring them. It is implemented here only so the demo
works.

## Phases

| # | Phase | Output | Gate |
|---|---|---|---|
| P0 | **Platform bring-up:** pick the upstream WIP combination that builds; run Trezor's sample app (Ethereum) end to end in the T3W1 emulator | a worktree on our fork, a recipe, exact commits | sample app loads and signs in the emulator |
| P1 | **Zcash key service** in coreapp on our branch | the service plus its protobuf and header entitlement; unit and device tests | adversarial and clarity reviews |
| P2 | **Zcash extapp** (Rust) | the app using the `ironwood` crate: `GetAddress`, `GetViewingKey`, streamed `SignPczt`, on SDK screens | emulator device tests at parity with the series (receive, viewing key, 2–32-action signs, deshield, memos, the recipient-address rule), plus real size measured |
| P3 | **Host** | trezorlib `zcash` over `ExtAppLoad`/`ExtAppMessage`; our wallet and engine using it | regtest end to end in the emulator |
| P4 | **Safe 7 hardware** (when it arrives) | an `--apps` dev build flashed on the dedicated test Safe 7 | receive, viewing key and send on the device |
| P5 | **Safe 5 alpha** | on Trezor's T3T1 arena WIP; fit in 231 KiB measured | the same on our Safe 5 |
| P6 | **Package** | branch, demo recipe, video, one-page summary, message draft | the user decides on contact |

## Carried over from the series (fixed as we go)

- The **recipient-address rule** (`dcc602fd48`) and every approval rule in
  `docs/common/zcash-ironwood-signing.md`. The rules live in the `ironwood` crate, which moves
  into the app unchanged.
- **The warm-up regression.** `prewarm()` calls `address_at`, which relinks FF1, `num_bigint`
  and `libm` (~16–18 KB). Fix it in the crate so the app is smaller too.
- **Moot in the app model:**
  - the 5 s chunk timer (transfer goes through `ExtAppMessage`);
  - the feature-off frozen-Python leak and the AUX1 region (the app has its own heap);
  - the `sign_pczt.py` structure findings (rewritten in Rust).

## Standing constraints

- **Dev keys only:** emulator and devel builds. Production ring keys do not exist upstream yet.
- **Installing on the test devices:** covered by the user's standing authorization. The Safe 7
  bootloader unlock is a separate, irreversible decision the user makes when it arrives.
- **No contact, PR or message to Trezor** without the user's explicit approval.

## App identity (user, 2026-09-25)

The manifest's vendor is "Bryant Wolf" and its app id is `zcash.trezor.com`. The user confirmed
both. Trezor may assign its own id at admission.

## Hardware log

- **2026-09-25, Safe 7, firmware `d485d458c5`.** The app loaded in 6.9 s, with the kernel
  ML-DSA root-packet check. After the user allowed the account, Core stopped with "unwrap
  failed". The cause was upstream `run.py` calling a debug-only logger in an optimised build.
  Fixed in `0245f15fd0` (see `EXTAPP_P4_PANIC.md`).
- **2026-09-26, Safe 7, firmware `0245f15fd0`, diagnostic app.**
  - **Load:** OK (6.9 s cold, 0.3 s resident).
  - **Receive:** testnet account 0 address #3 **MATCH**. Heap peak 40,076 of 73,744 B;
    longest IPC silence 285 ms.
  - **Viewing key:** **MATCH**. Heap peak 38,228 B; longest silence 259 ms.
  - **Signing:** both the 2- and 32-action signs failed with "Timeout waiting for message"
    about 3 s after the weak-backup warning. Signing start runs over Core's 1 s IPC watchdog.
    The fix is in progress (`EXTAPP_P4_WATCHDOG.md`).
- **2026-09-26, Safe 7, firmware `0245f15fd0` (not reflashed), diagnostic app from bundle
  `extapp-07a27d1d76`** (stack-frame split, stepped progress; `EXTAPP_P4_WATCHDOG.md`).
  **All steps MATCH**; log `session-logs/2026-09-26-safe7-extapp-session-2.log`.
  - **Load:** OK (186.0 KB upload, 5.9 s).
  - **Receive:** testnet account 0 address #3 **MATCH**. Heap peak 40,076 of 73,736 B;
    longest IPC silence 286 ms.
  - **Viewing key:** **MATCH**. Heap peak 38,228 B; longest silence 259 ms.
  - **2-action sign** (synthetic mainnet fixture, nothing broadcast): records **MATCH**.
    Heap peak 57,448 B; longest silence 213 ms; 38 IPC messages.
  - **32-action sign:** 16 payments, 32 recipient screens and the total; records **MATCH**.
    Heap peak 57,452 B (16,284 B spare); longest silence 198 ms; 393 IPC messages.
  - Core's 1 s IPC watchdog is unchanged, and no tables are precomputed. The worst IPC silence
    in the session was 286 ms.
