use core::ffi::CStr;

use ironwood::receive::{derive_external_receiver, derive_full_viewing_key, Network};
use ironwood::{Memo, MAX_ACTIONS};

use crate::ironwood::allocator;
use crate::ironwood::signing::{self, Failure, Step, RECORD_LEN};
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

        // pasta_curves' `hash_to_curve` boxes its hasher, so receiver and
        // viewing-key derivation allocate. Root (or reuse) the rooted tier
        // first: no scratch is installed outside a signing session, so these
        // blocks -- and any lazy static they fill -- land there, which is where
        // anything that outlives the call has to be.
        allocator::install_region();

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

        // pasta_curves' `hash_to_curve` boxes its hasher, so receiver and
        // viewing-key derivation allocate. Root (or reuse) the rooted tier
        // first: no scratch is installed outside a signing session, so these
        // blocks -- and any lazy static they fill -- land there, which is where
        // anything that outlives the call has to be.
        allocator::install_region();

        // SAFETY: The seed is immutable, the output is a distinct writable
        // object, and neither reference is retained or crosses into Python.
        let seed = unsafe { get_buffer(args[0])? };
        let output = unsafe { get_buffer_mut(args[3])? };
        let output: &mut [u8; 96] = output
            .try_into()
            .map_err(|_| Error::ValueError(c"Invalid viewing key output length"))?;

        derive_full_viewing_key(seed, network, account, output)
            .map_err(|_| Error::RuntimeError(c"Viewing key derivation failed"))?;

        // Cross-check against `orchard`, the implementation that signs. The
        // key just written comes from this firmware's own ZIP-32 and Orchard
        // key expansion; the spending path uses `orchard`'s. Nothing but
        // goldens has ever held the two together, and a disagreement would be
        // silent and expensive: the wallet would receive to addresses derived
        // from one key while every real spend failed the session's byte-for-
        // byte `fvk` check against the other. Refuse to export rather than
        // export something that cannot be spent from.
        let signing_network = match network {
            Network::Mainnet => ironwood::Network::Mainnet,
            Network::Testnet => ironwood::Network::Testnet,
        };
        let independent = signing::orchard_full_viewing_key(seed, signing_network, account)
            .map_err(|_| Error::RuntimeError(c"Viewing key derivation failed"))?;
        let mut difference = 0u8;
        for (a, b) in output.iter().zip(independent.iter()) {
            difference |= a ^ b;
        }
        if difference != 0 {
            output.fill(0);
            return Err(Error::RuntimeError(c"Viewing key derivation failed"));
        }
        Ok(Obj::const_none())
    };

    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

extern "C" fn seed_fingerprint(seed: Obj, output: Obj) -> Obj {
    let block = || {
        // Requiring immutable bytes prevents the seed and writable output from
        // aliasing while both are borrowed by Rust.
        if !unsafe { ffi::mp_type_bytes.is_type_of(seed) } {
            return Err(Error::TypeError);
        }
        // SAFETY: The seed is immutable, the output is a distinct writable
        // object, and neither reference is retained or crosses into Python.
        let seed = unsafe { get_buffer(seed)? };
        let output = unsafe { get_buffer_mut(output)? };
        let output: &mut [u8; 32] = output
            .try_into()
            .map_err(|_| Error::ValueError(c"Invalid seed fingerprint output length"))?;
        signing::seed_fingerprint(seed, output)
            .map_err(|_| Error::RuntimeError(c"Seed fingerprint derivation failed"))?;
        Ok(Obj::const_none())
    };
    unsafe { util::try_or_raise(block) }
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
        // A well-formed transaction whose values, or their running totals, fall
        // outside the money range. Separate from `Capacity` so neither message
        // has to describe the other's bound.
        Failure::Amount => Error::ValueError(c"Zcash amount out of range"),
        Failure::Policy => Error::RuntimeError(c"PCZT violates device policy"),
        Failure::State => Error::RuntimeError(c"Invalid signing state"),
        Failure::Signing => Error::RuntimeError(c"Signing failed"),
    }
}

/// The handle `session_begin` returned. Never 0, so a caller that lost it
/// cannot pass a plausible default.
fn parse_handle(value: Obj) -> Result<u32, Error> {
    let handle = parse_u32(value, c"Invalid signing handle")?;
    if handle == 0 {
        return Err(Error::ValueError(c"Invalid signing handle"));
    }
    Ok(handle)
}

fn parse_network(value: Obj) -> Result<ironwood::Network, Error> {
    match parse_u32(value, c"Invalid network")? {
        0 => Ok(ironwood::Network::Mainnet),
        1 => Ok(ironwood::Network::Testnet),
        _ => Err(Error::ValueError(c"Invalid network")),
    }
}

extern "C" fn session_begin(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        if args.len() != 8 {
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
        // Nothing may be installed at entry. The workflow releases its scratch
        // in a `finally`, so a live one here means that `finally` never ran and
        // the buffer behind it is no longer ours to touch; see
        // `allocator::forbid_installed_scratch`.
        allocator::forbid_installed_scratch();
        // Any earlier request must release its blocks while the tier they were
        // carved from is still installed, and only then may that tier go. With
        // no scratch installed both calls are the idle no-op they look like,
        // but the order still matters if a future path installs one.
        signing::cancel(None);
        allocator::release_scratch();
        // Root (or reuse) the boot-lifetime rooted tier. It is a native
        // `.zcash_region` static, not a Python object, so it survives every
        // session for the whole boot -- which is what keeps the Pasta table and
        // the orchard OnceBox caches valid across sessions.
        allocator::install_region();
        // The persistent set has to be built while the rooted tier is the only
        // one installed: `prewarm` fills exactly the lazy statics that outlive
        // a session, and the allocator routes to the scratch as soon as there
        // is one. Once per boot; a later session finds it already there.
        ironwood::prewarm();
        let (scratch, scratch_len) = {
            // SAFETY: the buffer is only measured here. The pointer is handed
            // to the allocator below and stays valid for the session because
            // the caller keeps the `bytearray` referenced until it has called
            // `session_cancel`, and the MicroPython collector does not move
            // objects.
            let scratch = unsafe { get_buffer_mut(args[7])? };
            (scratch.as_mut_ptr(), scratch.len())
        };
        // SAFETY: as above -- the caller owns the buffer for the whole session.
        if !unsafe { allocator::install_scratch(scratch, scratch_len) } {
            return Err(Error::ValueError(c"Invalid signing scratch buffer"));
        }
        // SAFETY: the seed is borrowed for this call only and not mutated.
        let seed = unsafe { get_buffer(args[0])? };
        let handle = signing::begin(
            seed,
            network,
            account,
            host_reference_height,
            maximum_fee,
            expiry_window,
            declared_len,
        )
        .map_err(|error| {
            // Nothing was carved that outlives this call, so give the scratch
            // back now rather than leaving it pinned until the teardown runs.
            allocator::release_scratch();
            failure(error)
        })?;
        Obj::try_from(handle)
    };

    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

extern "C" fn session_feed(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        if args.len() != 2 {
            return Err(Error::TypeError);
        }
        let handle = parse_handle(args[0])?;
        // SAFETY: the chunk is borrowed for the call only and not mutated.
        let chunk = unsafe { get_buffer(args[1])? };
        let (consumed, step) = signing::feed(handle, chunk).map_err(failure)?;
        let (kind, payload): (u8, Obj) = match step {
            Step::Continue => (0, Obj::const_none()),
            Step::Output(output) => {
                // Memo kinds as the handler shows them: 0 nothing, 1 text
                // (UTF-8 bytes within the budget), 2 the 32-byte digest of a
                // memo that is not shown verbatim. Mirrored on the Python
                // side by `_MEMO_NONE` / `_MEMO_TEXT` / `_MEMO_DIGEST` in
                // core/src/apps/zcash/sign_pczt.py; the two lists must move
                // together.
                let (memo_kind, memo): (u8, Obj) = match &output.memo {
                    Memo::Empty => (0, Obj::const_none()),
                    Memo::Text(text) => (1, Obj::try_from(text.as_str().as_bytes())?),
                    Memo::Digest(digest) => (2, Obj::try_from(&digest[..])?),
                };
                (
                    1,
                    Tuple::alloc(&[
                        Obj::try_from(output.action_index)?,
                        Obj::try_from(&output.receiver[..])?,
                        Obj::try_from(output.value)?,
                        Obj::from(memo_kind),
                        memo,
                    ])?
                    .into(),
                )
            }
            Step::TransparentOutput(output) => (
                3,
                Tuple::alloc(&[
                    Obj::try_from(output.index)?,
                    Obj::from(output.kind),
                    Obj::try_from(&output.hash[..])?,
                    Obj::try_from(output.value)?,
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
                    Obj::try_from(totals.transparent_total)?,
                    Obj::try_from(totals.fee)?,
                    Obj::try_from(totals.padding_outputs)?,
                    Obj::try_from(totals.payment_outputs)?,
                    Obj::try_from(totals.transparent_outputs)?,
                    Obj::try_from(totals.action_count)?,
                ])?
                .into(),
            ),
        };
        Ok(Tuple::alloc(&[Obj::try_from(consumed)?, Obj::from(kind), payload])?.into())
    };

    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

extern "C" fn session_approve(handle: Obj) -> Obj {
    let block = || {
        signing::approve(parse_handle(handle)?).map_err(failure)?;
        Ok(Obj::const_none())
    };
    unsafe { util::try_or_raise(block) }
}

extern "C" fn session_sign(handle: Obj, seed: Obj) -> Obj {
    let block = || {
        let handle = parse_handle(handle)?;
        if !unsafe { ffi::mp_type_bytes.is_type_of(seed) } {
            return Err(Error::TypeError);
        }
        let mut records = [0u8; MAX_ACTIONS * RECORD_LEN];
        let count = {
            // SAFETY: the seed is borrowed for the derivation only.
            let seed = unsafe { get_buffer(seed)? };
            signing::sign(handle, seed, &mut records).map_err(failure)?
        };
        Obj::try_from(&records[..count * RECORD_LEN])
    };
    unsafe { util::try_or_raise(block) }
}

extern "C" fn session_cancel(n_args: usize, args: *const Obj) -> Obj {
    let block = |args: &[Obj], _kwargs: &Map| {
        let handle = match args.first() {
            None => None,
            Some(value) if *value == Obj::const_none() => None,
            Some(value) => Some(parse_handle(*value)?),
        };
        signing::cancel(handle);
        // The scratch tier goes back to the collector exactly when nothing is
        // left to carve from it. A cancel that refused a foreign handle left
        // the session live and must not take its memory away.
        if signing::is_idle() {
            allocator::release_scratch();
        }
        Ok(Obj::const_none())
    };
    unsafe { util::try_with_args_and_kwargs(n_args, args, &Map::EMPTY, block) }
}

/// `(persist_in_use, persist_peak, scratch_in_use, scratch_peak)` on a
/// debuglink DEVICE build, `None` everywhere else: the arena bookkeeping is a
/// device instrument, not firmware telemetry, and nothing outside the debug
/// app reads it.
///
/// `None` on the emulator is not an omission. `allocator_unix.rs` is plain
/// `malloc` and installs no tier, so the only figures it could offer are four
/// zeros -- and four zeros read as a passing measurement of invariants
/// (`scratch_in_use == 0`, `persist_in_use` unchanged) that nothing there
/// actually holds. Absent says what is true: there are no arenas here.
extern "C" fn debug_region_info() -> Obj {
    let block = || {
        #[cfg(all(feature = "debuglink", target_arch = "arm"))]
        {
            let (persist_in_use, persist_peak, scratch_in_use, scratch_peak) =
                allocator::region_info();
            Ok(Tuple::alloc(&[
                Obj::try_from(persist_in_use)?,
                Obj::try_from(persist_peak)?,
                Obj::try_from(scratch_in_use)?,
                Obj::try_from(scratch_peak)?,
            ])?
            .into())
        }
        #[cfg(not(all(feature = "debuglink", target_arch = "arm")))]
        Ok(Obj::const_none())
    };
    unsafe { util::try_or_raise(block) }
}

const _: () = assert!(allocator::SCRATCH_BYTES <= u16::MAX as usize);

#[no_mangle]
#[rustfmt::skip]
pub static mp_module_trezorironwood: Module = obj_module! {
    // Bytes of per-session scratch `session_begin` requires, mirrored by
    // `apps.zcash.sign_pczt.SCRATCH_BYTES`. A small int, so asserted to fit
    // below: past 65,535 the cast would wrap and Python would read a tier
    // smaller than the one `install_scratch` demands.
    /// SCRATCH_BYTES: int
    Qstr::MP_QSTR_SCRATCH_BYTES => Obj::small_int(allocator::SCRATCH_BYTES as u16),

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
    /// def seed_fingerprint(seed: bytes, output: AnyBuffer) -> None:
    ///     """Fill a 32-byte buffer with the ZIP-32 seed fingerprint of the wallet
    ///     seed: a public identifier of the seed, not key material."""
    Qstr::MP_QSTR_seed_fingerprint => obj_fn_2!(seed_fingerprint).as_obj(),
    /// def session_begin(
    ///     seed: bytes,
    ///     network: int,
    ///     account: int,
    ///     host_reference_height: int,
    ///     maximum_fee: int,
    ///     expiry_window: int,
    ///     pczt_length: int,
    ///     scratch: AnyBuffer,
    /// ) -> int:
    ///     """Start streaming one PCZT for the account derived from the wallet seed.
    ///     Returns the session handle, which `session_feed`, `session_approve` and
    ///     `session_sign` require: it binds the native request to the workflow that
    ///     began it, so a second request cannot adopt this one.
    ///     `scratch` is a writable buffer of at least SCRATCH_BYTES that the
    ///     caller must keep referenced until it has called `session_cancel`,
    ///     and must not expose: the session's own allocations are carved from
    ///     it, so for the length of the session it holds the signing state,
    ///     the hedge secret, the viewing key and the decrypted note
    ///     plaintexts, and any Python holding the reference can read them.
    ///     What must outlive the session stays in a boot-lifetime native
    ///     region instead. `session_cancel` wipes the buffer before it gives
    ///     it back; calling `session_begin` again without it is fatal.
    ///     ValueError: the scratch buffer is too small."""
    Qstr::MP_QSTR_session_begin => obj_fn_var!(8, 8, session_begin).as_obj(),
    /// def session_feed(handle: int, chunk: AnyBytes) -> tuple[int, int, tuple | None]:
    ///     """Consume PCZT bytes. Returns (consumed, kind, payload): kind 0 needs more
    ///     bytes; kind 1 is a payment output to confirm, payload
    ///     (action_index, receiver, value, memo_kind, memo) where
    ///     memo_kind 0 is no memo (memo None), 1 a text memo (memo: its UTF-8
    ///     bytes, at most 256) and 2 a memo not shown verbatim (memo: the 32-byte
    ///     BLAKE2b-256 of the memo); kind 2 is the review, payload
    ///     (expiry_height, blocks_until_expiry, input_total, payment_total,
    ///     change_total, transparent_total, fee, padding_outputs,
    ///     payment_outputs, transparent_outputs, action_count); kind 3 is a
    ///     transparent output to confirm, payload (index, kind, hash, value)
    ///     where kind 0 is a public key hash and 1 a script hash, and hash is
    ///     the 20 bytes the Base58Check address encodes.
    ///     Unconsumed bytes must be fed again. A kind 3 may consume nothing:
    ///     transparent outputs are held until the action count is known, and
    ///     are then released one per call. ValueError: malformed / too many
    ///     actions; RuntimeError: rejected."""
    Qstr::MP_QSTR_session_feed => obj_fn_var!(2, 2, session_feed).as_obj(),
    /// def session_approve(handle: int) -> None:
    ///     """Record consent for the reviewed PCZT; call only after the trusted totals screen."""
    Qstr::MP_QSTR_session_approve => obj_fn_1!(session_approve).as_obj(),
    /// def session_sign(handle: int, seed: bytes) -> bytes:
    ///     """Sign every real spend and end the session. Returns concatenated
    ///     66-byte records: pool (0x03) | action_index | signature[64]."""
    Qstr::MP_QSTR_session_sign => obj_fn_2!(session_sign).as_obj(),
    /// def session_cancel(handle: int | None = None) -> None:
    ///     """End the session and wipe its state. With a handle, only that
    ///     handle's session is ended, so a workflow cannot tear down a session
    ///     that is no longer its own. Without one, whatever is live is ended:
    ///     teardown runs from a `finally` that may have no handle yet, and
    ///     autolock unwinds the workflow with a GeneratorExit from outside.
    ///     Once nothing is live the scratch buffer is wiped and given back, so
    ///     the caller may drop its reference after this returns."""
    Qstr::MP_QSTR_session_cancel => obj_fn_var!(0, 1, session_cancel).as_obj(),
    /// def debug_region_info() -> tuple[int, int, int, int] | None:
    ///     """(persist_in_use, persist_peak, scratch_in_use, scratch_peak) of
    ///     the two signing arenas on a debuglink DEVICE build, None otherwise
    ///     -- including on the emulator, which has no arenas to report. An
    ///     absent reading must be skipped, not read as zeros."""
    Qstr::MP_QSTR_debug_region_info => obj_fn_0!(debug_region_info).as_obj(),
};
