#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

extern crate alloc;

use alloc::vec::Vec;

use prost::Message;
use trezor_app_sdk::{
    Error, Result, ResultExt, WireDecode, WireEncode, error, wire_error_raw, wire_handler,
    wire_receive_wire_start,
};

// Include generated code
pub(crate) mod proto;

#[macro_use]
pub(crate) mod translations;

mod get_address;
mod get_viewing_key;
mod helpers;
mod keys;
mod layout;
mod paths;
mod sign_pczt;
mod strutil;
mod unified;

use proto::{
    messages::MessageType,
    zcash::{GetAddress, GetViewingKey, SignPczt},
};

/// Wire codec for [`wire_handler!`]: encodes and decodes messages with prost.
struct ProstCodec;

impl<T: Message + Default> WireDecode<T> for ProstCodec {
    fn decode(data: &[u8]) -> Result<T> {
        T::decode(data).map_err(|_| Error::InvalidMessage)
    }
}

impl<T: Message> WireEncode<T> for ProstCodec {
    fn encode(val: &T) -> Vec<u8> {
        val.encode_to_vec()
    }
}

wire_handler!(
    handle_get_address,
    ProstCodec,
    GetAddress,
    MessageType::Address,
    get_address::get_address
);
wire_handler!(
    handle_get_viewing_key,
    ProstCodec,
    GetViewingKey,
    MessageType::ViewingKey,
    get_viewing_key::get_viewing_key
);
wire_handler!(
    handle_sign_pczt,
    ProstCodec,
    SignPczt,
    MessageType::SpendAuthSignatures,
    sign_pczt::sign_pczt
);

#[unsafe(no_mangle)]
pub fn app() -> Result<()> {
    loop {
        let (id, data) = wire_receive_wire_start().c()?;
        handle_wire_message(id, &data).c()?;
    }
}

fn handle_wire_message(id: u16, data: &[u8]) -> Result<()> {
    match MessageType::try_from(i32::from(id)) {
        Ok(MessageType::GetAddress) => handle_get_address(data),
        Ok(MessageType::GetViewingKey) => handle_get_viewing_key(data),
        Ok(MessageType::SignPczt) => handle_sign_pczt(data),
        // No signing is in progress; answered as Core answers `Cancel`.
        Ok(MessageType::Cancel) => wire_error_raw(&Error::Cancelled),
        _ => {
            error!("Unexpected message type: {}", id);
            Err(Error::InvalidFunction)
        }
    }
}
