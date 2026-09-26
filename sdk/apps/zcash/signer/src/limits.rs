//! Size limits of an accepted PCZT.

/// Largest PCZT, in bytes.
pub const MAX_PCZT_BYTES: usize = 65_536;

/// Most actions in one PCZT: Ironwood actions plus transparent outputs.
pub const MAX_ACTIONS: usize = 32;

/// Transparent outputs count against [`MAX_ACTIONS`], as ZIP-317 counts them,
/// and at least one Ironwood action is required.
pub const MAX_TRANSPARENT_OUTPUTS: usize = MAX_ACTIONS - 1;

/// Longest `user_address`; covers a unified address with every receiver.
pub const MAX_USER_ADDRESS_BYTES: usize = 512;

/// P2PKH `scriptPubKey`: `OP_DUP OP_HASH160 <20> OP_EQUALVERIFY OP_CHECKSIG`.
pub(crate) const P2PKH_SCRIPT_BYTES: usize = 25;

/// P2SH `scriptPubKey`: `OP_HASH160 <20> OP_EQUAL`.
pub(crate) const P2SH_SCRIPT_BYTES: usize = 23;

pub(crate) const MAX_SCRIPT_PUBKEY_BYTES: usize = P2PKH_SCRIPT_BYTES;
