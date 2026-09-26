//! Memos, recovered from the signed ciphertext and shown as text, as a
//! digest, or not at all (ZIP 302).

use zcash_protocol::memo::MemoBytes;
use zcash_signer::Error::{Malformed, Policy};
use zcash_signer::{MAX_MEMO_TEXT_BYTES, Memo, OutputKind};
use zcash_signer_tests::*;

/// The memo shown for a payment carrying `memo`. The change carries none.
fn shown(wallet: &Wallet, memo: &[u8]) -> Memo {
    let tx = Tx::pay(
        vec![
            Output {
                memo: MemoBytes::from_bytes(memo).unwrap(),
                ..Output::payment(600_000)
            },
            Output::change(390_000),
        ],
        Vec::new(),
        10_000,
    );
    let events = review(wallet, &build(wallet, &tx).bytes, 1024).unwrap();
    let change = events
        .review()
        .summary()
        .outputs
        .iter()
        .find(|o| o.kind == OutputKind::InternalChange);
    assert_eq!(change.unwrap().memo, Memo::Empty);
    events.payments[0].0.memo.clone()
}

/// A memo of `first` followed by `rest` up to 512 bytes.
fn with_first(first: &[u8], rest: u8) -> Vec<u8> {
    let mut memo = vec![rest; 512];
    memo[..first.len()].copy_from_slice(first);
    memo
}

/// Each memo is shown as the BLAKE2b-256 of its 512 bytes.
fn assert_digests(memos: &[(&str, Vec<u8>)]) {
    let wallet = Wallet::testnet();
    for (name, memo) in memos {
        let digest = memo_digest(&MemoBytes::from_bytes(memo).unwrap());
        assert_eq!(shown(&wallet, memo), Memo::Digest(digest), "{name}");
    }
}

/// ZIP 302's empty memo, in either encoding.
#[test]
fn test_memo_not_shown() {
    let wallet = Wallet::testnet();
    for (name, memo) in [("empty", vec![0xf6]), ("zeros", vec![0; 512])] {
        assert_eq!(shown(&wallet, &memo), Memo::Empty, "{name}");
    }
}

#[test]
fn test_memo_as_text() {
    let wallet = Wallet::testnet();
    for (name, text) in [
        ("short", "hello"),
        ("UTF-8", "Zodl ✓ café ☕ — thanks!"),
        ("line break", "line one\nline two"),
        ("CJK", "\u{4F60}\u{597D}"),
        ("at the limit", &"a".repeat(MAX_MEMO_TEXT_BYTES)),
    ] {
        match shown(&wallet, text.as_bytes()) {
            Memo::Text(shown) => assert_eq!(shown.as_str(), text, "{name}"),
            memo => panic!("{name}: {memo:?}"),
        }
    }
}

/// Memos that are not text of at most `MAX_MEMO_TEXT_BYTES`.
#[test]
fn test_memo_as_digest() {
    assert_digests(&[
        ("past the limit", vec![b'a'; MAX_MEMO_TEXT_BYTES + 1]),
        ("512 bytes of text", vec![b'a'; 512]),
        ("not UTF-8", vec![0xc3, 0x28]),
        ("arbitrary data", with_first(&[0xff], 0x41)),
        ("reserved", with_first(&[0xf5], 0)),
        ("future format", with_first(&[0xf6, 1], 0)),
        ("interior NUL", b"a\0b".to_vec()),
    ]);
}

/// Text the screen would not draw as written.
#[test]
fn test_unrenderable_text_as_digest() {
    let texts = [
        ("outside the BMP", "pay me \u{1F600}"),
        ("aliases 'A' in 16 bits", "\u{10041}BC"),
        ("right-to-left override", "send 1 ZEC to \u{202E}bob"),
        ("embedding and pop", "a\u{202A}b\u{202C}c"),
        ("zero width space", "tre\u{200B}zor"),
        ("zero width joiner", "a\u{200D}b"),
        ("word joiner", "a\u{2060}b"),
        ("isolates", "a\u{2066}b\u{2069}c"),
        ("line separator", "a\u{2028}b"),
        ("paragraph separator", "a\u{2029}b"),
        ("soft hyphen", "co\u{00AD}op"),
        ("no-break space", "a\u{00A0}b"),
        ("byte order mark", "\u{FEFF}hello"),
        ("variation selector", "x\u{FE0F}"),
        ("carriage return", "a\rb"),
        ("tab", "a\tb"),
        ("delete", "a\u{7F}b"),
        ("next line", "a\u{0085}b"),
    ];
    let memos: Vec<_> = texts
        .into_iter()
        .map(|(name, text)| (name, text.as_bytes().to_vec()))
        .collect();
    assert_digests(&memos);
}

/// Outputs that are not shown must not carry a memo.
#[test]
fn test_hidden_memo() {
    let wallet = Wallet::testnet();
    let memo = MemoBytes::from_bytes(b"hidden").unwrap();
    let cases = [
        (
            "change",
            vec![
                Output::payment(600_000),
                Output {
                    memo: memo.clone(),
                    ..Output::change(390_000)
                },
            ],
        ),
        (
            "zero payment",
            vec![
                Output {
                    memo: memo.clone(),
                    ..Output::payment(0)
                },
                Output::change(990_000),
            ],
        ),
    ];
    for (name, outputs) in cases {
        let bytes = build(&wallet, &Tx::pay(outputs, Vec::new(), 10_000)).bytes;
        assert_outcome(&wallet, name, &bytes, Err(Policy));
    }
}

/// The memo shown is the one encrypted in the signed ciphertext.
#[test]
fn test_memo_bound_to_ciphertext() {
    let wallet = Wallet::testnet();
    let tx = Tx::pay(
        vec![
            Output {
                memo: MemoBytes::from_bytes(b"hello").unwrap(),
                ..Output::payment(600_000)
            },
            Output::change(390_000),
        ],
        Vec::new(),
        10_000,
    );
    let bytes = build(&wallet, &tx).bytes;
    // The memo follows the 52-byte note plaintext.
    let mutated = mutate(&bytes, |v| {
        let ciphertext = &mut payment_action(v)["output"]["enc_ciphertext"]["Encrypted"];
        ciphertext[60] = (ciphertext[60].as_u64().unwrap() ^ 1).into();
    });
    assert_outcome(&wallet, "memo byte", &mutated, Err(Malformed));
}
