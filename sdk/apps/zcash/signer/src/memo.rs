//! Recovery of an output's memo from its signed ciphertext, and how it is
//! shown (ZIP 302).

use orchard::Note;
use orchard::keys::{FullViewingKey, Scope};
use orchard::note_encryption::IronwoodDomain;
use zcash_note_encryption::Domain;

use crate::error::{Error, Result, ensure};

const MEMO_BYTES: usize = 512;

const EMPTY_MEMO: [u8; MEMO_BYTES] = {
    let mut memo = [0; MEMO_BYTES];
    memo[0] = 0xf6;
    memo
};
const PADDING_MEMO: [u8; MEMO_BYTES] = [0; MEMO_BYTES];

/// Longest memo text shown verbatim; a longer memo is shown as its hash.
pub const MAX_MEMO_TEXT_BYTES: usize = 256;

/// How a payment's memo is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
// Inline, so that reviewing an output does not allocate.
#[allow(clippy::large_enum_variant)]
pub enum Memo {
    /// No memo: the `0xF6` marker, or empty text.
    Empty,
    /// Text the device draws as written.
    Text(MemoText),
    /// Any other memo: the BLAKE2b-256 of all 512 bytes, shown as hex.
    Digest([u8; 32]),
}

/// The text of a memo; valid UTF-8.
#[derive(Clone, PartialEq, Eq)]
pub struct MemoText {
    len: u16,
    bytes: [u8; MAX_MEMO_TEXT_BYTES],
}

impl MemoText {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

impl core::fmt::Debug for MemoText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_str(), f)
    }
}

/// Whether the device draws `c` as itself. A memo with any other character is
/// shown as its hash, so that the screen cannot differ from what is signed.
fn renders_faithfully(c: char) -> bool {
    let code = c as u32;
    // The glyph lookup truncates code points to 16 bits.
    if code > 0xFFFF {
        return false;
    }
    // '\n' breaks the line; other controls are not drawn as written.
    if c != '\n' && c.is_control() {
        return false;
    }
    !matches!(
        code,
        // No-break space, drawn as a plain space.
        0x00A0
        // Invisible marks, which can hide or fuse their neighbours.
        | 0x00AD | 0x034F | 0x061C | 0x180E | 0xFE00..=0xFE0F | 0xFEFF | 0xFFF9..=0xFFFB
        // Zero-width and bidirectional controls, and line separators, which
        // can reorder what is drawn.
        | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x2069
    )
}

fn classify(memo: &[u8; MEMO_BYTES]) -> Memo {
    if *memo == EMPTY_MEMO || *memo == PADDING_MEMO {
        return Memo::Empty;
    }
    // A first byte up to 0xF4 is UTF-8 text, padded with zeros.
    if memo[0] <= 0xF4 {
        let end = memo.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        let text = &memo[..end];
        if text.len() <= MAX_MEMO_TEXT_BYTES
            && !text.contains(&0)
            && core::str::from_utf8(text).is_ok_and(|s| s.chars().all(renders_faithfully))
        {
            let mut bytes = [0; MAX_MEMO_TEXT_BYTES];
            bytes[..text.len()].copy_from_slice(text);
            return Memo::Text(MemoText {
                len: text.len() as u16,
                bytes,
            });
        }
    }
    let mut digest = [0; 32];
    digest.copy_from_slice(
        blake2b_simd::Params::new()
            .hash_length(32)
            .hash(memo)
            .as_bytes(),
    );
    Memo::Digest(digest)
}

/// Checks the output's note encryption under the account's keys and returns
/// its memo as shown; padding and change must carry an empty memo. `note` is
/// the note whose commitment matched the action's `cmx`.
pub(crate) fn recover(
    action: &orchard::pczt::Action,
    fvk: &FullViewingKey,
    outgoing_scope: Scope,
    note: &Note,
    progress: &mut dyn FnMut(),
) -> Result<Memo> {
    let output = action.output();
    let domain = IronwoodDomain::for_pczt_action(action);
    // Recovery checks rho and the note version against `note` itself.
    let memo = orchard::note_encryption::recover_output_bound_with_pkd_esk(
        &domain,
        IronwoodDomain::get_pk_d(note),
        IronwoodDomain::derive_esk(note).ok_or(Error::Malformed)?,
        action,
        note,
    )
    .ok_or(Error::Malformed)?;
    let shown = if note.value().inner() == 0 {
        ensure(memo == EMPTY_MEMO || memo == PADDING_MEMO, Error::Policy)?;
        Memo::Empty
    } else if outgoing_scope == Scope::Internal {
        ensure(memo == EMPTY_MEMO, Error::Policy)?;
        Memo::Empty
    } else {
        classify(&memo)
    };
    let out_ciphertext = &output.encrypted_note().out_ciphertext;
    if let Some(ock) = output.ock() {
        progress();
        ensure(
            orchard::note_encryption::recover_output_bound_with_ock(
                &domain,
                ock,
                action,
                out_ciphertext,
                note,
            )
            .is_some_and(|m| m == memo),
            Error::Malformed,
        )?;
    }
    if note.value().inner() > 0 {
        let mut recovers_under = |scope| {
            progress();
            orchard::note_encryption::recover_output_bound_with_ovk(
                &domain,
                &fvk.to_ovk(scope),
                action,
                action.cv_net(),
                out_ciphertext,
                note,
            )
            .is_some_and(|m| m == memo)
        };
        match outgoing_scope {
            // A payment must stay recoverable under the external OVK.
            Scope::External => ensure(recovers_under(Scope::External), Error::Malformed)?,
            // Change may omit the OVK, but must not be recoverable under the
            // external one.
            Scope::Internal => {
                if !recovers_under(Scope::Internal) {
                    ensure(!recovers_under(Scope::External), Error::Malformed)?;
                }
            }
        }
    }
    // Padding built without an OVK has a random `out_ciphertext`; it is still
    // covered by the sighash and the signatures.
    Ok(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo(text: &[u8]) -> [u8; MEMO_BYTES] {
        let mut memo = [0; MEMO_BYTES];
        memo[..text.len()].copy_from_slice(text);
        memo
    }

    fn digest(memo: &[u8; MEMO_BYTES]) -> Memo {
        let mut digest = [0; 32];
        digest.copy_from_slice(
            blake2b_simd::Params::new()
                .hash_length(32)
                .hash(memo)
                .as_bytes(),
        );
        Memo::Digest(digest)
    }

    #[test]
    fn test_empty_memos() {
        assert_eq!(classify(&EMPTY_MEMO), Memo::Empty);
        assert_eq!(classify(&PADDING_MEMO), Memo::Empty);
    }

    #[test]
    fn test_text_memos() {
        let texts: [&str; 4] = ["hello", "line\nbreak", "Příliš žluťoučký kůň", "日本語"];
        for text in texts {
            match classify(&memo(text.as_bytes())) {
                Memo::Text(shown) => assert_eq!(shown.as_str(), text),
                other => panic!("{text:?}: {other:?}"),
            }
        }
        let longest = [b'a'; MAX_MEMO_TEXT_BYTES];
        assert!(matches!(classify(&memo(&longest)), Memo::Text(_)));
    }

    #[test]
    fn test_memos_shown_as_digest() {
        let too_long = [b'a'; MAX_MEMO_TEXT_BYTES + 1];
        let cases: [&[u8]; 9] = [
            &too_long,
            b"\xff binary",
            b"\xf5 reserved",
            b"\xc3\x28 not utf-8",
            b"zero\0inside",
            "carriage\rreturn".as_bytes(),
            "emoji \u{1F600}".as_bytes(),
            "no-break\u{a0}space".as_bytes(),
            "bidi \u{202e}override".as_bytes(),
        ];
        for text in cases {
            let memo = memo(text);
            assert_eq!(classify(&memo), digest(&memo), "{text:?}");
        }
    }

    #[test]
    fn test_renders_faithfully() {
        for c in ['a', 'Z', ' ', '\n', 'ž', '€', '\u{FFFD}'] {
            assert!(renders_faithfully(c), "{c:?}");
        }
        let refused = [
            '\t',
            '\r',
            '\u{7f}',
            '\u{85}',
            '\u{a0}',
            '\u{ad}',
            '\u{34f}',
            '\u{61c}',
            '\u{180e}',
            '\u{fe0f}',
            '\u{feff}',
            '\u{fffb}',
            '\u{200b}',
            '\u{200f}',
            '\u{2028}',
            '\u{202e}',
            '\u{2060}',
            '\u{2069}',
            '\u{10000}',
            '\u{1f600}',
        ];
        for c in refused {
            assert!(!renders_faithfully(c), "{c:?}");
        }
    }
}
