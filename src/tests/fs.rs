use super::*;
use std::path::PathBuf;

#[test]
fn test_is_btrfs_nonexistent() {
    // Should not panic on non-existent path; the bool value depends on the
    // root filesystem type, so we only assert that the call returns cleanly.
    let _result = is_btrfs(&PathBuf::from("/nonexistent/path/12345"));
}

#[test]
fn test_is_btrfs_root() {
    // Should work on root
    let _result = is_btrfs(&PathBuf::from("/"));
    // Just verify it doesn't panic
}
