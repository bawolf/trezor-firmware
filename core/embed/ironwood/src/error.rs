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
