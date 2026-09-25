//! Zcash shielded (Orchard/Ironwood) app: receive addresses and viewing keys.
//!
//! The account's key material comes from [`account::account_keys`]; all
//! Pallas work is done here with the `ironwood` crate.

#![no_std]
#![no_main]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

extern crate alloc;

use alloc::vec::Vec;

use prost::Message;
use trezor_app_sdk::{
    Error, Result, ResultExt, WireDecode, WireEncode, error, wire_handler, wire_receive_wire_start,
};

#[macro_use]
mod translations;
#[macro_use]
mod strutil;

mod account;
#[cfg(feature = "dev-test-seed")]
mod dev_test_seed;
mod get_address;
mod get_viewing_key;
mod layout;
mod proto;
mod unified;

use proto::{
    messages::MessageType,
    zcash::{ZcashGetAddress, ZcashGetViewingKey},
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
    ZcashGetAddress,
    MessageType::ZcashAddress,
    get_address::get_address
);
wire_handler!(
    handle_get_viewing_key,
    ProstCodec,
    ZcashGetViewingKey,
    MessageType::ZcashViewingKey,
    get_viewing_key::get_viewing_key
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
        Ok(MessageType::ZcashGetAddress) => handle_get_address(data),
        Ok(MessageType::ZcashGetViewingKey) => handle_get_viewing_key(data),
        _ => {
            error!("Unexpected message type: {}", id);
            Err(Error::InvalidFunction)
        }
    }
}
