/// Stable approval-core error classes for the application adapter.
///
/// Deliberately carries no diagnostic string: detailed text would enlarge the
/// firmware interface without being stable or actionable at the transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorCode {
    Malformed,
    Policy,
    State,
    Capacity,
    /// A value, or a running total of values, is outside the money range.
    /// Split from `Capacity` so the handler can say which bound was hit: a
    /// transaction whose amounts sum past MAX_MONEY is not "too many actions".
    Amount,
    Entropy,
    Signing,
    Internal,
}

impl ErrorCode {
    pub(crate) const fn malformed() -> Self {
        Self::Malformed
    }

    pub(crate) const fn policy() -> Self {
        Self::Policy
    }

    pub(crate) const fn state() -> Self {
        Self::State
    }

    pub(crate) const fn capacity() -> Self {
        Self::Capacity
    }

    pub(crate) const fn amount() -> Self {
        Self::Amount
    }

    pub(crate) const fn entropy() -> Self {
        Self::Entropy
    }

    pub(crate) const fn signing() -> Self {
        Self::Signing
    }

    pub(crate) const fn internal() -> Self {
        Self::Internal
    }

    pub const fn code(self) -> Self {
        self
    }
}

/// Natural result-error spelling without a second representation.
pub type Error = ErrorCode;
pub type Result<T> = core::result::Result<T, ErrorCode>;
