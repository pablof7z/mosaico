use super::*;
use nostr::{Keys, ToBech32};
use std::io::Cursor;

#[test]
fn owner_accepts_hex_and_npub_and_normalizes_to_hex() {
    let public_key = Keys::generate().public_key();
    let expected = public_key.to_hex();
    assert_eq!(resolve_owner(Some(expected.clone())).unwrap(), expected);
    assert_eq!(
        resolve_owner(Some(public_key.to_bech32().unwrap())).unwrap(),
        expected
    );
}

#[test]
fn owner_rejects_invalid_input() {
    let error = resolve_owner(Some("not-a-pubkey".into())).unwrap_err();
    assert!(error.to_string().contains("invalid relay owner public key"));
}

#[test]
fn owner_falls_back_to_first_whitelisted_operator() {
    let first = Keys::generate().public_key().to_hex();
    let second = Keys::generate().public_key().to_hex();

    assert_eq!(
        select_owner(None, vec![first.clone(), second]).unwrap(),
        first
    );
}

#[test]
fn owner_requires_explicit_or_whitelisted_operator() {
    let error = select_owner(None, Vec::new()).unwrap_err();
    assert!(error.to_string().contains("pass --owner-pubkey"));
}

#[test]
fn embedded_archive_extraction_restores_executable_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("croissant");
    let mut archive = Vec::new();
    zstd::stream::copy_encode(Cursor::new(b"relay-binary"), &mut archive, 1).unwrap();

    extract_archive(&archive, &target).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"relay-binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            target.metadata().unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}
