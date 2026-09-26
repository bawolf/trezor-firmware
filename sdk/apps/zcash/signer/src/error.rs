/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The PCZT does not parse, or does not verify.
    Malformed,
    /// The PCZT is valid but not something the device signs.
    Policy,
    /// A call out of order, or with a different key.
    State,
    /// More actions or bytes than the device accepts.
    Capacity,
    /// A value or sum of values exceeds `MAX_MONEY`.
    Amount,
    /// The random number generator failed.
    Entropy,
    /// The spend authorizing key does not match the PCZT.
    Signing,
    /// A broken internal invariant.
    Internal,
}

pub type Result<T> = core::result::Result<T, Error>;

pub(crate) fn ensure(ok: bool, error: Error) -> Result<()> {
    if ok { Ok(()) } else { Err(error) }
}
