use alloc::string::String;
use core::convert::Infallible;

use ufmt::uWrite;

/// A `ufmt` writer into a `String`, for [`uformat!`].
pub(crate) struct StringWriter(pub(crate) String);

impl uWrite for StringWriter {
    type Error = Infallible;

    fn write_str(&mut self, s: &str) -> Result<(), Self::Error> {
        self.0.push_str(s);
        Ok(())
    }
}

/// Like `format!`, with `ufmt` (as in the Ethereum and Tron apps).
macro_rules! uformat {
    ($($tt:tt)*) => {{
        use trezor_app_sdk::unwrap;
        let mut s = $crate::strutil::StringWriter(alloc::string::String::new());
        unwrap!(ufmt::uwrite!(&mut s, $($tt)*));
        s.0
    }};
}
