use super::*;

#[test]
fn symlinked_mosaico_pty_parent_refuses_without_touching_its_destination() {
    let fixture = tempfile::tempdir().unwrap();
    let destination = fixture.path().join("external-runtime");
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("keep.sock"), b"unrelated state").unwrap();
    let predictable_parent = fixture.path().join("mosaico-pty-501");
    std::os::unix::fs::symlink(&destination, &predictable_parent).unwrap();

    let error = audit_pty_socket_directory(&predictable_parent.join("selected-home"))
        .expect_err("symlinked Mosaico PTY parent must refuse reset");

    assert!(error.to_string().contains("symlinked runtime target"));
    assert_eq!(
        std::fs::read(destination.join("keep.sock")).unwrap(),
        b"unrelated state"
    );
}

#[test]
fn filesystem_root_is_never_a_selected_mosaico_home() {
    let root = Path::new(std::path::MAIN_SEPARATOR_STR);
    let error = audit_selected_root(root, &[]).expect_err("root must refuse reset");
    assert!(error.to_string().contains("dangerously broad selected"));
}
