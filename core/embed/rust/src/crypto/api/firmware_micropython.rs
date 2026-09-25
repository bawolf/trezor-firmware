#[cfg(feature = "app_loading")]
use core::mem::MaybeUninit;

#[cfg(feature = "app_loading")]
use rkyv::{
    api::low::to_bytes_in_with_alloc,
    rancor::Failure,
    ser::{allocator::SubAllocator, writer::Buffer},
    util::Align,
    Archived,
};
#[cfg(feature = "app_loading")]
use trezor_app_sdk::crypto::{
    access_crypto_request, Slice, TrezorCryptoEnum, TrezorCryptoResultRef,
};
#[cfg(feature = "app_loading")]
use zeroize::Zeroize;

#[cfg(feature = "app_loading")]
use crate::micropython::gc::Gc;
use crate::micropython::macros::{obj_fn_kw, obj_module};
use crate::micropython::map::Map;
use crate::micropython::module::Module;
use crate::micropython::obj::Obj;
use crate::micropython::qstr::Qstr;
#[cfg(feature = "app_loading")]
use crate::micropython::{error::Error, list::List, util};

#[cfg(feature = "app_loading")]
extern "C" fn new_deserialize_crypto_message(
    n_args: usize,
    args: *const Obj,
    kwargs: *mut Map,
) -> Obj {
    let block = |_args: &[Obj], kwargs: &Map| {
        let obj: Obj = kwargs.get(Qstr::MP_QSTR_data)?;
        let message_id: u16 = kwargs.get(Qstr::MP_QSTR_message_id)?.try_into()?;

        let data = unwrap!(unsafe { crate::micropython::buffer::get_buffer(obj) });

        fn obj_from_dp_slice(slice: &Archived<Slice<u32>>) -> Obj {
            let slice = slice.as_ref();
            let mut list = unwrap!(List::with_capacity(slice.len()));

            for &item in slice {
                unwrap!(list.append(unwrap!(Obj::try_from(item.to_native()))));
            }
            unwrap!(List::alloc(unsafe { list.as_slice() })).into()
        }

        // The request comes from an untrusted app: validate it before reading.
        let archived = access_crypto_request(data, message_id)
            .ok_or(Error::ValueError(c"Invalid crypto request"))?;

        // Access the archived data zero-copy using safe Deref access
        let result: Obj = match archived {
            Archived::<TrezorCryptoEnum>::GetXpub {
                address_n,
                xpub_magic,
            } => {
                let dp_obj = obj_from_dp_slice(address_n);
                let magic_obj = unwrap!(Obj::try_from(xpub_magic.to_native()));
                (dp_obj, magic_obj).try_into()?
            }
            Archived::<TrezorCryptoEnum>::GetPublicKey {
                address_n,
                compressed,
            } => {
                let dp_obj = obj_from_dp_slice(address_n);
                let compressed_obj = unwrap!(Obj::try_from(*compressed));
                (dp_obj, compressed_obj).try_into()?
            }
            Archived::<TrezorCryptoEnum>::SignDigest {
                address_n,
                digest,
                compressed,
            } => {
                let digest_obj = Obj::try_from(digest.as_slice())?;
                (
                    obj_from_dp_slice(address_n),
                    digest_obj,
                    Obj::try_from(*compressed)?,
                )
                    .try_into()?
            }
            Archived::<TrezorCryptoEnum>::SignTypedHash {
                address_n,
                hash,
                encoded_network,
                encoded_token,
                chain_id,
                show_progress,
            } => {
                let hash_obj = Obj::try_from(hash.as_slice())?;
                let network_obj = match encoded_network.as_ref() {
                    Some(network) => Obj::try_from(network.as_ref())?,
                    None => Obj::const_none(),
                };
                let token_obj = match encoded_token.as_ref() {
                    Some(token) => Obj::try_from(token.as_ref())?,
                    None => Obj::const_none(),
                };
                let chain_id_obj = match chain_id.as_ref() {
                    Some(id) => Obj::try_from(id.to_native())?,
                    None => Obj::const_none(),
                };
                (
                    obj_from_dp_slice(address_n),
                    hash_obj,
                    network_obj,
                    token_obj,
                    chain_id_obj,
                    Obj::try_from(*show_progress)?,
                )
                    .try_into()?
            }
            Archived::<TrezorCryptoEnum>::GetAddressMac { address_n, address } => (
                obj_from_dp_slice(address_n),
                Obj::try_from(address.as_ref())?,
            )
                .try_into()?,
            Archived::<TrezorCryptoEnum>::VerifyNonceCache { nonce } => {
                Obj::try_from(nonce.as_ref())?
            }
            Archived::<TrezorCryptoEnum>::CheckAddressMac {
                address_n,
                mac,
                address,
            } => (
                obj_from_dp_slice(address_n),
                Obj::try_from(mac.as_ref())?,
                Obj::try_from(address.as_ref())?,
            )
                .try_into()?,
            Archived::<TrezorCryptoEnum>::GetZip32OrchardAccount { coin_type, account } => (
                Obj::try_from(coin_type.to_native())?,
                Obj::try_from(account.to_native())?,
            )
                .try_into()?,
        };

        Ok(result)
    };
    unsafe { util::try_with_args_and_kwargs(n_args, args, kwargs, block) }
}

#[cfg(not(feature = "app_loading"))]
extern "C" fn new_deserialize_crypto_message(
    _n_args: usize,
    _args: *const Obj,
    _kwargs: *mut Map,
) -> Obj {
    unimplemented!()
}

/// A crypto result whose spending key, if any, is zeroed when dropped.
#[cfg(feature = "app_loading")]
struct WipedResult<'a>(TrezorCryptoResultRef<'a>);

#[cfg(feature = "app_loading")]
impl Drop for WipedResult<'_> {
    fn drop(&mut self) {
        if let TrezorCryptoResultRef::Zip32OrchardAccount { spending_key, .. } = &mut self.0 {
            spending_key.zeroize();
        }
    }
}

/// Serialization buffers, zeroed when dropped: they may hold a spending key.
#[cfg(feature = "app_loading")]
struct WipedBuffers {
    arena: [MaybeUninit<u8>; 200],
    out: Align<[MaybeUninit<u8>; 200]>,
}

#[cfg(feature = "app_loading")]
impl Drop for WipedBuffers {
    fn drop(&mut self) {
        self.arena.zeroize();
        self.out.0.zeroize();
    }
}

#[cfg(feature = "app_loading")]
extern "C" fn new_send_crypto_result(n_args: usize, args: *const Obj, kwargs: *mut Map) -> Obj {
    let block = |_args: &[Obj], kwargs: &Map| {
        let obj: Obj = kwargs.get(Qstr::MP_QSTR_result)?;

        let ipc_callback: Option<Obj> = kwargs
            .get(Qstr::MP_QSTR_ipc_cb)
            .unwrap_or_else(|_| Obj::const_none())
            .try_into_option()?;

        let ipc_cb = unwrap!(ipc_callback.map(|cb| {
            move |bytes: &[u8]| {
                unwrap!(cb.call_with_n_args(&[unwrap!(bytes.try_into())]));
            }
        }));

        // Map MicroPython CryptoResult object to Rust enum for serialization
        let msg = WipedResult(if obj.is_str() {
            let data = unwrap!(unsafe { crate::micropython::buffer::get_buffer(obj) });
            match data.len() {
                111 => TrezorCryptoResultRef::Xpub(unwrap!(data.try_into())),
                _ => {
                    return Err(Error::TypeError);
                }
            }
        } else if obj.is_bytes() {
            let data = unwrap!(unsafe { crate::micropython::buffer::get_buffer(obj) });
            match data.len() {
                32 => TrezorCryptoResultRef::AddressMac(unwrap!(data.try_into())),
                65 => TrezorCryptoResultRef::Signature(unwrap!(data.try_into())),
                _ => {
                    return Err(Error::TypeError);
                }
            }
        } else if obj.is_immediate() {
            TrezorCryptoResultRef::Boolean(unwrap!(bool::try_from(obj)))
        } else {
            // Expect a `[type_tag: int, ...]` list for results that a byte
            // length cannot identify (e.g. 32 or 65 bytes)
            let list: Gc<List> = obj.try_into()?;
            let tag: u8 = unwrap!(list.get(0)?.try_into());
            let bytes_at = |index: usize| -> Result<&[u8], Error> {
                let item = list.get(index)?;
                unsafe { crate::micropython::buffer::get_buffer(item) }
            };
            match (tag, list.len()) {
                // [0, public_key]
                (0, 2) => {
                    let data = bytes_at(1)?;
                    assert!(
                        data.len() == 32 || data.len() == 33 || data.len() == 65,
                        "Expected public key to be 32, 33 or 65 bytes"
                    );
                    TrezorCryptoResultRef::PublicKey(unwrap!(data.try_into()))
                }
                // [1, spending_key, seed_fingerprint, weak_backup]
                (1, 4) => {
                    let seed_fingerprint = bytes_at(2)?.try_into().map_err(|_| Error::TypeError)?;
                    let weak_backup = bool::try_from(list.get(3)?)?;
                    // Copied last, so no error path leaves an unwiped copy.
                    let spending_key = bytes_at(1)?.try_into().map_err(|_| Error::TypeError)?;
                    TrezorCryptoResultRef::Zip32OrchardAccount {
                        spending_key,
                        seed_fingerprint,
                        weak_backup,
                    }
                }
                // [2]
                (2, 1) => TrezorCryptoResultRef::Cancelled,
                _ => {
                    return Err(Error::TypeError);
                }
            }
        });

        let mut buffers = WipedBuffers {
            arena: [MaybeUninit::<u8>::uninit(); 200],
            out: Align([MaybeUninit::<u8>::uninit(); 200]),
        };

        let bytes = unwrap!(to_bytes_in_with_alloc::<_, _, Failure>(
            &msg.0,
            Buffer::from(&mut *buffers.out),
            SubAllocator::new(&mut buffers.arena),
        ));
        //Send the response back via the ipc_cb callback
        ipc_cb(bytes.as_ref());

        Ok(Obj::const_none())
    };
    unsafe { util::try_with_args_and_kwargs(n_args, args, kwargs, block) }
}

#[cfg(not(feature = "app_loading"))]
extern "C" fn new_send_crypto_result(_n_args: usize, _args: *const Obj, _kwargs: *mut Map) -> Obj {
    unimplemented!()
}

#[no_mangle]
pub static mp_module_trezorcrypto_api: Module = obj_module! {

    /// mock:global
    Qstr::MP_QSTR___name__ => Qstr::MP_QSTR_trezorcrypto_api.to_obj(),

    /// def send_crypto_result(
    ///     *,
    ///     result: CryptoResult,
    ///     ipc_cb: Callable[[bytes], None],
    /// ) -> None:
    ///     """Serialize a crypto result (e.g. CryptoResult) into bytes and send it back via the ipc_cb callback."""
    Qstr::MP_QSTR_send_crypto_result => obj_fn_kw!(0, new_send_crypto_result).as_obj(),



    /// def deserialize_crypto_message(
    ///     *,
    ///     data: bytes,
    ///     message_id: int,
    /// ) -> Obj:
    ///     """Validate a crypto request for operation `message_id` and return its
    ///     fields as a MicroPython object. Raises ValueError if it is malformed or
    ///     another operation."""
    Qstr::MP_QSTR_deserialize_crypto_message => obj_fn_kw!(0, new_deserialize_crypto_message).as_obj(),
};
