//! Arbitrary corruption: the Session never accepts a PCZT that librustzcash
//! refuses, and whatever it signs verifies with librustzcash.

use serde_json::json;
use zcash_signer_tests::*;

/// Every single-byte change of a PCZT that uses most of the grammar: a
/// transparent output, the Full view's Sapling bundle and binding keys, an
/// anchor, a `user_address`, a derivation claim and a dummy spend.
#[test]
fn test_single_byte_changes() {
    let wallet = Wallet::testnet();
    let tx = Tx {
        view: View::Full,
        ..Tx::pay(
            vec![Output {
                user_address: Some("u1recipient".into()),
                ..Output::payment(600_000)
            }],
            transparent_outputs(1),
            10_000,
        )
    };
    let built = build(&wallet, &tx);
    let claim = derivation_json(&wallet.seed_fingerprint, &wallet.account_path());
    let bytes = mutate(&built.bytes, |v| {
        v["ironwood"]["anchor"] = json!(vec![0u8; 32]);
        v["ironwood"]["actions"][built.spends[0]]["spend"]["zip32_derivation"] = claim;
    });
    for (offset, &byte) in bytes.iter().enumerate() {
        for changed in [byte ^ 0x01, byte ^ 0x80] {
            let mut mutated = bytes.clone();
            mutated[offset] = changed;
            if review(&wallet, &mutated, usize::MAX).is_ok() {
                assert!(
                    librustzcash_verifies(&mutated, &wallet.fvk),
                    "byte {offset} = {changed}"
                );
                assert_signatures_apply(&mutated, &sign(&wallet, &mutated).unwrap());
            }
        }
    }
}
