# The Zcash extapp fits the Safe 5 arena

Date 2026-09-26. Branch `zcash/extapp-safe5` @ `36b839dfea` (local; its content is being
reshaped into the upstream series). Full report and receipts: the author's
`.context/product-scaffold/safe5-fit/`.

## Result

| Image | code | RW | stack | heap | Total | Spare |
|---|---|---|---|---|---|---|
| **Safe 5 (T3T1) release** | 121,728 | 2,208 | 32,768 | 69,632 | **226,336** | **+10,208 of 236,544** |
| Safe 5 debug | 124,608 | 2,232 | 32,768 | 69,632 | 229,248 | +7,296 |
| Safe 7 (T3W1) release | 187,872 | 2,208 | 32,768 | 69,632 | 292,480 | +100,736 of 393,216 |
| Before (`07a27d1d76`) | 187,456 | 16,544 | 32,768 | 73,728 | 310,496 | −73,952 on the Safe 5 |

The arena is Trezor's WIP T3T1 arena (`534e35daa9`, the same content as `adb18e1795` on
`modular-ethereum-wip`): 231 KiB at `0x3006A000`. Code and data both count.

## What changed

- **IPC inbox, −14,336 B.** A new SDK manifest key `ipc-buffer-size`; the Zcash app declares
  2 KiB. The largest message it receives is a 1 KiB chunk (1,049 B + a 12 B header). A host
  reply that does not fit now stops the app (the fix and a test are in the same stack).
- **Computed Sinsemilla generators on the Safe 5 only, −66,112 B.** Progress is reported
  inside every Sinsemilla hash (fork branches `ironwood/sinsemilla-progress`: orchard `61704d4`,
  sinsemilla `6607cb0`, pushed to `bawolf`).
- **Heap 72 → 68 KiB, −4,096 B.** The emulator now grants exactly the declared heap. The full
  suite passes at 60,416 B and fails at 59,904 B. The sign peak is 57,600 B (hardware Safe 7:
  57,452 B).
- **Unchanged:** Core's 1 s watchdog, `run.py` timing, every check and screen. No precomputed
  table was added.

## Margins

- **Heap:** 9,216 B (15 %) above the smallest working heap.
- **Stack:** the static worst chain is 29,744 B, including literal-pool calls. With 512 B for
  API stubs and exception frames, that leaves ≥ 2,512 B. Not measured on a device.
- **IPC silence:** estimated worst 0.26–0.36 s on a Safe 5, against 1 s.
- **Signing time:** a 32-action sign is ≈100–170 s of verification on a Safe 5 (≈3.4× the
  table build). **An open product question for the user.**

## Tests

- **Device tests:** 59 passed on the T3W1 and T3T1 emulators, with the 68 KiB heap enforced.
- **Crate tests:** 126 passed with each generator setting.
- **Style:** clean.
- **Reviews:** Fable 5.1 adversarial ×2, with one real bug (an oversized host chunk hung the
  app) fixed with a test. Readability review by Opus, applied.

## Open

- A Safe 5 hardware session with a diagnostic image. The T3T1 `--apps` firmware builds at FLASH
  95.16 %. It has not been flashed.
- Stack painting on a device.
