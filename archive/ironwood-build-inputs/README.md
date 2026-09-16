# Archived Ironwood experiment source closure

**Archive only — unsafe — not for a pull request or production use.**

This directory is the reviewable, repository-contained dependency closure for the
experimental Safe 5 reconstruction based on Trezor commit
`7105338e3c2c1e681940e17780609881ce53126b`. It preserves hard-coded regtest/account-9
policy, diagnostic endpoints, and obsolete split-GC/fixed-arena work. None of it is
ancestry for the upstream-shaped implementation.

The source was copied from the immutable `fork-preservation-01` build-input archive.
Nested `.git` metadata was deliberately excluded; librustzcash provenance (upstream
revision, status and exact patch) remains in that preservation record. Three exact
host-test files were recovered from `safe5-wallet-composition-01` after matching the
bridge source hash. The stale lifecycle host test was updated only for the existing
borrowed-seed ABI and its synthetic entropy/clear symbols; production bridge behavior
was not changed.

`ironwood-approval` originated in project commits `7da3bc22f7e26122511dbe4044d0a6fdaea6c378`
and `12c25c8eb9d9045483a2dbfebc0483a779bd0944`. The bridge baseline originated in
project commit `ce8d217223b2a9d5e9d9ca8527fb1ff91d7bd061`. Both declare MIT and carry the
project `LICENSE` notice adjacent to their manifests. All copied upstream Trezor and
Zcash license files remain in place.
