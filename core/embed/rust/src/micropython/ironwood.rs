use core::ffi::CStr;

use ironwood::receive::{derive_external_receiver, derive_full_viewing_key, Network};

use crate::ironwood::{allocator, signing};
use crate::micropython::buffer::{get_buffer, get_buffer_mut};
use crate::micropython::map::Map;
use crate::micropython::module::Module;
use crate::micropython::qstr::Qstr;
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
            use crate::micropython::tuple::Tuple;

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
    /// def debug_region_info() -> tuple[int, int, int, int] | None:
    ///     """(persist_in_use, persist_peak, scratch_in_use, scratch_peak) of
    ///     the two signing arenas on a debuglink DEVICE build, None otherwise
    ///     -- including on the emulator, which has no arenas to report. An
    ///     absent reading must be skipped, not read as zeros."""
    Qstr::MP_QSTR_debug_region_info => obj_fn_0!(debug_region_info).as_obj(),
};
