use core::ffi::CStr;

use trezor_ironwood_receive::{derive_external_receiver, derive_full_viewing_key, Network};

use crate::micropython::buffer::{get_buffer, get_buffer_mut};
use crate::micropython::ffi;
use crate::micropython::map::Map;
use crate::micropython::module::Module;
use crate::micropython::qstr::Qstr;
use crate::micropython::{util, Error, Obj};

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
};
