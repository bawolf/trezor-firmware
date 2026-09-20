use core::ffi::CStr;

use trezor_ironwood::MAX_ACTIONS;
use trezor_ironwood_receive::{derive_external_receiver, derive_full_viewing_key, Network};

use crate::ironwood_signing::{self, Failure, Step, RECORD_LEN};
use crate::micropython::buffer::{get_buffer, get_buffer_mut};
use crate::micropython::map::Map;
use crate::micropython::module::Module;
use crate::micropython::qstr::Qstr;
use crate::micropython::tuple::Tuple;
use crate::micropython::{ffi, util, Error, Obj};

fn parse_u32(value: Obj, range_message: &'static CStr) -> Result<u32, Error> {
    u32::try_from(value).map_err(|error| match error {
        Error::OutOfRange => Error::ValueError(range_message),
        other => other,
    })
}

extern "C" fn derive_receiver(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        if args.len() != 4 {
            return Err(Error::TypeError);
        }

        let network = parse_u32(args[1], c"Invalid network")?;
        let network =
            Network::try_from(network).map_err(|_| Error::ValueError(c"Invalid network"))?;
        let account = parse_u32(args[2], c"Invalid account")?;
        if account > 0x7fff_ffff {
            return Err(Error::ValueError(c"Invalid account"));
        }

        let receiver = {
            // SAFETY: No MicroPython code or allocation runs while either buffer is
            // borrowed. Neither buffer is mutated, and no reference is retained.
            let index = unsafe { get_buffer(args[3])? };
            let diversifier_index: [u8; 11] = index
                .try_into()
                .map_err(|_| Error::ValueError(c"Invalid diversifier index length"))?;
            let seed = unsafe { get_buffer(args[0])? };

            derive_external_receiver(seed, network, account, diversifier_index)
                .map_err(|_| Error::RuntimeError(c"Receiver derivation failed"))?
        };

        receiver.as_slice().try_into()
    };

    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

extern "C" fn derive_viewing_key(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        if args.len() != 4 {
            return Err(Error::TypeError);
        }

        let network = parse_u32(args[1], c"Invalid network")?;
        let network =
            Network::try_from(network).map_err(|_| Error::ValueError(c"Invalid network"))?;
        let account = parse_u32(args[2], c"Invalid account")?;
        if account > 0x7fff_ffff {
            return Err(Error::ValueError(c"Invalid account"));
        }
        // Requiring immutable bytes prevents the seed and writable output from
        // aliasing while both are borrowed by Rust.
        if !unsafe { ffi::mp_type_bytes.is_type_of(args[0]) } {
            return Err(Error::TypeError);
        }

        // SAFETY: The seed is immutable, the output is a distinct writable
        // object, and neither reference is retained or crosses into Python.
        let seed = unsafe { get_buffer(args[0])? };
        let output = unsafe { get_buffer_mut(args[3])? };
        let output: &mut [u8; 96] = output
            .try_into()
            .map_err(|_| Error::ValueError(c"Invalid viewing key output length"))?;

        derive_full_viewing_key(seed, network, account, output)
            .map_err(|_| Error::RuntimeError(c"Viewing key derivation failed"))?;
        Ok(Obj::const_none())
    };

    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

/// Python raises `DataError` for `ValueError` and `ProcessError` for
/// `RuntimeError`, so only malformed bytes become a `ValueError`.
fn failure(failure: Failure) -> Error {
    match failure {
        Failure::Malformed => Error::ValueError(c"Malformed PCZT"),
        // A well-formed but oversized transaction (more than the 32-action cap,
        // or over the byte limit). Its own comprehensible reason, not a raw
        // "malformed": the handler surfaces this message to the user.
        Failure::Capacity => Error::ValueError(c"Too many transaction actions (max 32)"),
        Failure::Policy => Error::RuntimeError(c"PCZT violates device policy"),
        Failure::State => Error::RuntimeError(c"Invalid signing state"),
        Failure::Signing => Error::RuntimeError(c"Signing failed"),
    }
}

fn parse_network(value: Obj) -> Result<trezor_ironwood::Network, Error> {
    match parse_u32(value, c"Invalid network")? {
        0 => Ok(trezor_ironwood::Network::Mainnet),
        1 => Ok(trezor_ironwood::Network::Testnet),
        _ => Err(Error::ValueError(c"Invalid network")),
    }
}

extern "C" fn session_begin(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        if args.len() != 7 {
            return Err(Error::TypeError);
        }
        let network = parse_network(args[1])?;
        let account = parse_u32(args[2], c"Invalid account")?;
        let host_reference_height = parse_u32(args[3], c"Invalid host reference height")?;
        let maximum_fee = u64::try_from(args[4])?;
        let expiry_window = parse_u32(args[5], c"Invalid expiry window")?;
        let declared_len = usize::try_from(args[6])?;
        // The seed must be immutable bytes.
        if !unsafe { ffi::mp_type_bytes.is_type_of(args[0]) } {
            return Err(Error::TypeError);
        }
        // Any earlier request must release its blocks before the new one begins.
        ironwood_signing::cancel();
        // Root (or reuse) the boot-lifetime signing region. It is a native
        // `.buf` static, not a Python object, so nothing needs to be kept
        // referenced across the session and it survives every session for the
        // whole boot (fixes cross-session staleness of the Pasta table / orchard
        // OnceBox caches; docs/decisions/2026-09-18-cross-session-region-lifetime.md).
        crate::ironwood_allocator::install_region();
        // Measurement bookkeeping: reset the per-session region peak and sample
        // the bytes already in use (the persistent Pasta table + orchard OnceBox
        // caches after session 1) BEFORE `begin` allocates, so the sweep can see a
        // flat per-session peak and a constant in-use-at-begin (a leak otherwise).
        crate::ironwood_allocator::mark_session_begin();
        // SAFETY: the seed is borrowed for this call only and not mutated.
        let seed = unsafe { get_buffer(args[0])? };
        ironwood_signing::begin(
            seed,
            network,
            account,
            host_reference_height,
            maximum_fee,
            expiry_window,
            declared_len,
        )
        .map_err(failure)?;
        Ok(Obj::const_none())
    };

    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

extern "C" fn session_feed(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        if args.len() != 1 {
            return Err(Error::TypeError);
        }
        // SAFETY: the chunk is borrowed for the call only and not mutated.
        let chunk = unsafe { get_buffer(args[0])? };
        let (consumed, step) = ironwood_signing::feed(chunk).map_err(failure)?;
        let (kind, payload): (u8, Obj) = match step {
            Step::Continue => (0, Obj::const_none()),
            Step::Output(output) => (
                1,
                Tuple::alloc(&[
                    Obj::try_from(output.action_index)?,
                    Obj::try_from(&output.receiver[..])?,
                    Obj::try_from(output.value)?,
                    Obj::from(output.is_change),
                ])?
                .into(),
            ),
            Step::Review(totals) => (
                2,
                Tuple::alloc(&[
                    Obj::try_from(totals.expiry_height)?,
                    Obj::try_from(totals.blocks_until_expiry)?,
                    Obj::try_from(totals.input_total)?,
                    Obj::try_from(totals.payment_total)?,
                    Obj::try_from(totals.change_total)?,
                    Obj::try_from(totals.fee)?,
                    Obj::try_from(totals.padding_outputs)?,
                    Obj::try_from(totals.payment_outputs)?,
                    Obj::try_from(totals.action_count)?,
                ])?
                .into(),
            ),
        };
        Ok(Tuple::alloc(&[Obj::try_from(consumed)?, Obj::from(kind), payload])?.into())
    };

    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

extern "C" fn session_approve() -> Obj {
    let block = || {
        ironwood_signing::approve().map_err(failure)?;
        Ok(Obj::const_none())
    };
    unsafe { util::try_or_raise(block) }
}

extern "C" fn session_sign(seed: Obj) -> Obj {
    let block = || {
        if !unsafe { ffi::mp_type_bytes.is_type_of(seed) } {
            return Err(Error::TypeError);
        }
        let mut records = [0u8; MAX_ACTIONS * RECORD_LEN];
        let count = {
            // SAFETY: the seed is borrowed for the derivation only.
            let seed = unsafe { get_buffer(seed)? };
            ironwood_signing::sign(seed, &mut records).map_err(failure)?
        };
        Obj::try_from(&records[..count * RECORD_LEN])
    };
    unsafe { util::try_or_raise(block) }
}

extern "C" fn session_cancel() -> Obj {
    ironwood_signing::cancel();
    Obj::const_none()
}

/// MEASUREMENT-ONLY region telemetry. Gated behind `ironwood-measurement`
/// (default-OFF): the returned high-water marks are internal region layout
/// information and are excluded from production builds (Fable review R1/#2).
#[cfg(feature = "ironwood-measurement")]
extern "C" fn session_region_high_water() -> Obj {
    let block = || {
        // (per_session_peak, in_use_at_begin, boot_peak), all in bytes from the
        // region base; every element is 0 on the emulator. The per-session peak
        // is the figure the sweep reads (the boot-monotone peak can only grow).
        Ok(Tuple::alloc(&[
            Obj::try_from(crate::ironwood_allocator::region_session_high_water())?,
            Obj::try_from(crate::ironwood_allocator::region_in_use_at_begin())?,
            Obj::try_from(crate::ironwood_allocator::region_high_water())?,
        ])?
        .into())
    };
    unsafe { util::try_or_raise(block) }
}

/// MEASUREMENT-ONLY per-operation micro-benchmark. Runs `iters` iterations of a
/// single selected crypto operation over the same `ironwood-sinsemilla`
/// (computed generators) and `ironwood-pasta-curves` instances the signing path
/// links, and returns a folded accumulator of every result so nothing is
/// optimised away. The caller installs the region (Pasta/Sinsemilla allocate)
/// by passing a bytearray and times the call with `utime.ticks_ms`.
/// Selector: 0 warmup, 1 note_commitment, 2 sinsemilla_hash, 3 scalar_mul,
/// 4 commit_ivk. Changes no signing behavior; reached only through the reserved
/// all-`0xff` diversifier-index bench path in `get_address`. Gated behind
/// `ironwood-measurement` (default-OFF): production builds link no bench symbol
/// and expose no bench binding (Fable review R1).
#[cfg(feature = "ironwood-measurement")]
extern "C" fn bench(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        if args.len() != 3 {
            return Err(Error::TypeError);
        }
        let selector = parse_u32(args[0], c"Invalid bench selector")?;
        if !unsafe { ffi::mp_type_bytearray.is_type_of(args[1]) } {
            return Err(Error::TypeError);
        }
        let iters = parse_u32(args[2], c"Invalid bench iterations")?;
        // The signing region is now a boot-lifetime `.buf` static, so the
        // caller's bytearray (arg 1) is accepted for API compatibility but
        // ignored; the bench allocates from the same rooted region signing uses.
        crate::ironwood_allocator::install_region();
        let accumulator: u64 = match selector {
            0 => {
                trezor_ironwood::bench::warmup();
                0
            }
            1 => trezor_ironwood::bench::note_commitment(iters),
            2 => trezor_ironwood::bench::sinsemilla_hash(iters),
            3 => trezor_ironwood::bench::scalar_mul(iters),
            4 => trezor_ironwood::bench::commit_ivk(iters),
            _ => return Err(Error::ValueError(c"Invalid bench selector")),
        };
        Obj::try_from(accumulator)
    };
    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

// The module is defined twice under mutually-exclusive cfgs. The PRODUCTION
// variant (default) omits the MEASUREMENT-ONLY `session_region_high_water` and
// `bench` bindings so no bench symbol links and no region telemetry is reachable
// (Fable review R1). The MEASUREMENT variant (`ironwood-measurement`) adds those
// two bindings for on-device op-timing sweeps. The `obj_module!` macro cannot
// cfg individual entries, so the two shared-plus-extra variants are spelled out;
// keep the shared entries below in sync between the two.

/// PRODUCTION module (default): no MEASUREMENT-ONLY bindings.
#[cfg(not(feature = "ironwood-measurement"))]
#[no_mangle]
#[rustfmt::skip]
pub static mp_module_trezorironwood: Module = obj_module! {
    /// def derive_receiver(
    ///     seed: AnyBytes,
    ///     network: int,
    ///     account: int,
    ///     diversifier_index: AnyBytes,
    /// ) -> bytes:
    ///     """Internal synchronous adapter; seed must come from device wallet state."""
    Qstr::MP_QSTR_derive_receiver => obj_fn_var!(4, 4, derive_receiver).as_obj(),
    /// def derive_viewing_key(
    ///     seed: bytes,
    ///     network: int,
    ///     account: int,
    ///     output: AnyBuffer,
    /// ) -> None:
    ///     """Fill a 96-byte Orchard FVK buffer from device wallet state."""
    Qstr::MP_QSTR_derive_viewing_key => obj_fn_var!(4, 4, derive_viewing_key).as_obj(),
    /// def session_begin(
    ///     seed: bytes,
    ///     network: int,
    ///     account: int,
    ///     host_reference_height: int,
    ///     maximum_fee: int,
    ///     expiry_window: int,
    ///     pczt_length: int,
    /// ) -> None:
    ///     """Start streaming one PCZT for the account derived from the wallet seed.
    ///     Allocations of the signing core are carved from a boot-lifetime native
    ///     region (no caller-provided buffer)."""
    Qstr::MP_QSTR_session_begin => obj_fn_var!(7, 7, session_begin).as_obj(),
    /// def session_feed(chunk: AnyBytes) -> tuple[int, int, tuple | None]:
    ///     """Consume PCZT bytes. Returns (consumed, kind, payload): kind 0 needs more
    ///     bytes; kind 1 is a payment output to confirm, payload
    ///     (action_index, receiver, value, is_change); kind 2 is the review, payload
    ///     (expiry_height, blocks_until_expiry, input_total, payment_total,
    ///     change_total, fee, padding_outputs, payment_outputs, action_count).
    ///     Unconsumed bytes must be fed again. ValueError: malformed / too many
    ///     actions; RuntimeError: rejected."""
    Qstr::MP_QSTR_session_feed => obj_fn_var!(1, 1, session_feed).as_obj(),
    /// def session_approve() -> None:
    ///     """Record consent for the reviewed PCZT; call only after the trusted totals screen."""
    Qstr::MP_QSTR_session_approve => obj_fn_0!(session_approve).as_obj(),
    /// def session_sign(seed: bytes) -> bytes:
    ///     """Sign every real spend and end the session. Returns concatenated
    ///     66-byte records: pool (0x03) | action_index | signature[64]."""
    Qstr::MP_QSTR_session_sign => obj_fn_1!(session_sign).as_obj(),
    /// def session_cancel() -> None:
    ///     """End the session, if any, and wipe its state."""
    Qstr::MP_QSTR_session_cancel => obj_fn_0!(session_cancel).as_obj(),
};

/// MEASUREMENT module (`ironwood-measurement`): adds op-timing + region
/// telemetry bindings for on-device sweeps. NOT for production.
#[cfg(feature = "ironwood-measurement")]
#[no_mangle]
#[rustfmt::skip]
pub static mp_module_trezorironwood: Module = obj_module! {
    /// def derive_receiver(
    ///     seed: AnyBytes,
    ///     network: int,
    ///     account: int,
    ///     diversifier_index: AnyBytes,
    /// ) -> bytes:
    ///     """Internal synchronous adapter; seed must come from device wallet state."""
    Qstr::MP_QSTR_derive_receiver => obj_fn_var!(4, 4, derive_receiver).as_obj(),
    /// def derive_viewing_key(
    ///     seed: bytes,
    ///     network: int,
    ///     account: int,
    ///     output: AnyBuffer,
    /// ) -> None:
    ///     """Fill a 96-byte Orchard FVK buffer from device wallet state."""
    Qstr::MP_QSTR_derive_viewing_key => obj_fn_var!(4, 4, derive_viewing_key).as_obj(),
    /// def session_begin(
    ///     seed: bytes,
    ///     network: int,
    ///     account: int,
    ///     host_reference_height: int,
    ///     maximum_fee: int,
    ///     expiry_window: int,
    ///     pczt_length: int,
    /// ) -> None:
    ///     """Start streaming one PCZT for the account derived from the wallet seed.
    ///     Allocations of the signing core are carved from a boot-lifetime native
    ///     region (no caller-provided buffer)."""
    Qstr::MP_QSTR_session_begin => obj_fn_var!(7, 7, session_begin).as_obj(),
    /// def session_feed(chunk: AnyBytes) -> tuple[int, int, tuple | None]:
    ///     """Consume PCZT bytes. Returns (consumed, kind, payload): kind 0 needs more
    ///     bytes; kind 1 is a payment output to confirm, payload
    ///     (action_index, receiver, value, is_change); kind 2 is the review, payload
    ///     (expiry_height, blocks_until_expiry, input_total, payment_total,
    ///     change_total, fee, padding_outputs, payment_outputs, action_count).
    ///     Unconsumed bytes must be fed again. ValueError: malformed / too many
    ///     actions; RuntimeError: rejected."""
    Qstr::MP_QSTR_session_feed => obj_fn_var!(1, 1, session_feed).as_obj(),
    /// def session_approve() -> None:
    ///     """Record consent for the reviewed PCZT; call only after the trusted totals screen."""
    Qstr::MP_QSTR_session_approve => obj_fn_0!(session_approve).as_obj(),
    /// def session_sign(seed: bytes) -> bytes:
    ///     """Sign every real spend and end the session. Returns concatenated
    ///     66-byte records: pool (0x03) | action_index | signature[64]."""
    Qstr::MP_QSTR_session_sign => obj_fn_1!(session_sign).as_obj(),
    /// def session_cancel() -> None:
    ///     """End the session, if any, and wipe its state."""
    Qstr::MP_QSTR_session_cancel => obj_fn_0!(session_cancel).as_obj(),
    /// def session_region_high_water() -> tuple[int, int, int]:
    ///     """MEASUREMENT-ONLY (ironwood-measurement). Region measurement counters,
    ///     in bytes from the region base (all 0 on the emulator):
    ///     (per_session_peak, in_use_at_begin, boot_peak). per_session_peak resets
    ///     at each session_begin, so on an ascending-N single-boot sweep it stays
    ///     ~flat; in_use_at_begin is the persistent set already allocated when the
    ///     session began (constant in steady state — any drift is a cross-session
    ///     leak); boot_peak is the boot-monotone maximum."""
    Qstr::MP_QSTR_session_region_high_water => obj_fn_0!(session_region_high_water).as_obj(),
    /// def bench(selector: int, region: bytearray, iters: int) -> int:
    ///     """MEASUREMENT-ONLY (ironwood-measurement). Run `iters` iterations of one
    ///     crypto operation (0 warmup, 1 note_commitment, 2 sinsemilla_hash,
    ///     3 scalar_mul, 4 commit_ivk) over the signing path's Sinsemilla/Pallas
    ///     instances and return a folded accumulator so nothing is optimised away.
    ///     `region` is accepted for API compatibility but IGNORED: the bench
    ///     allocates from the same boot-lifetime `.buf` region the signing path
    ///     installs, so an empty bytearray() is fine. Time it with utime.ticks_ms
    ///     on the Python side. Changes no signing behavior."""
    Qstr::MP_QSTR_bench => obj_fn_var!(3, 3, bench).as_obj(),
};
