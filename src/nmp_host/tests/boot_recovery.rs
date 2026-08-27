//! Boot-recovery measurement stubs.
//!
//! These measurement tests existed to benchmark NMP's durable queue boot cost.
//! With fava the write store is RedbWriteStore; the equivalent measurement
//! would benchmark fava's own boot path. Keeping the stubs preserves the
//! module tree without re-testing fava internals.

#[test]
#[ignore = "measurement stub; boot-cost benchmarks belong in fava's own test suite"]
fn daemon_boot_over_a_large_publish_queue() {}

#[test]
#[ignore = "needs MOSAICO_BOOT_STORE; measurement stub"]
fn daemon_boot_over_an_existing_store() {}
