use alloc::string::String;
use core::{convert::Infallible, result::Result};

use ufmt::uWrite;

pub struct StringWriter(String);

impl StringWriter {
    pub fn new() -> Self {
        Self(String::new())
    }

    pub fn finalize(self) -> String {
        self.0
    }
}

impl uWrite for StringWriter {
    type Error = Infallible;

    fn write_str(&mut self, s: &str) -> Result<(), Self::Error> {
        self.0.push_str(s);
        Ok(())
    }
}

// Returns an `alloc::string::String` using `ufmt::uwrite!`
// from https://docs.rs/ufmt/latest/ufmt/
// like `std::format!` it returns a `String` but uses `uwrite!`
// instead of `write!`
#[macro_export]
macro_rules! uformat {
    ($($tt:tt)*) => {
        {
            use trezor_app_sdk::unwrap;
            let mut s = $crate::strutil::StringWriter::new();
            unwrap!(ufmt::uwrite!(&mut s, $($tt)*));
            s.finalize()
        }
    };
}

pub fn hex_encode(bytes: &[u8]) -> Result<String, ()> {
    let mut s = StringWriter::new();
    for byte in bytes {
        ufmt::uwrite!(&mut s, "{:02x}", *byte).map_err(|_| ())?;
    }
    Ok(s.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_encode_empty() {
        let result = hex_encode(&[]).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_hex_encode_multiple_bytes() {
        let result = hex_encode(&[0x00, 0xFF, 0xAB, 0xCD]).unwrap();
        assert_eq!(result, "00ffabcd");
    }

    #[test]
    fn test_uformat() {
        assert_eq!(uformat!("ZEC #{}", 1u32), "ZEC #1");
    }
}
