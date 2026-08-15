//! Contract tests for `sessionatlas_core::path`.
//!
//! Every case exercises an explicit flavor so both the Windows and the POSIX
//! contract run on any host; native-flavor cases only assert host-specific
//! behavior with host-appropriate inputs.

use sessionatlas_core::path::{self, PathFlavor};

#[test]
fn path_semantics_preserves_windows_roots_and_resolves_segments() {
    let cases: &[(&str, &str)] = &[
        (r"C:\", r"C:\"),
        (r"c:/Repo/", r"C:\Repo"),
        (r"C:\repo\.\child\..\", r"C:\repo"),
        (r"\\server\share", r"\\server\share"),
        (r"\\server\share\repo\", r"\\server\share\repo"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            path::normalize(PathFlavor::Windows, input).as_deref(),
            Some(*expected),
            "windows normalize {input:?}"
        );
    }
}

#[test]
fn path_semantics_preserves_unix_roots_and_resolves_segments() {
    let cases: &[(&str, &str)] = &[
        ("/", "/"),
        ("/repo/", "/repo"),
        ("/repo/./child/../", "/repo"),
        ("//", "/"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            path::normalize(PathFlavor::Unix, input).as_deref(),
            Some(*expected),
            "unix normalize {input:?}"
        );
    }
}

#[test]
fn path_semantics_dotdot_above_root_collapses_to_root() {
    assert_eq!(
        path::normalize(PathFlavor::Unix, "/..").as_deref(),
        Some("/")
    );
    assert_eq!(
        path::normalize(PathFlavor::Unix, "/a/../../b").as_deref(),
        Some("/b")
    );
    assert_eq!(
        path::normalize(PathFlavor::Windows, r"C:\..\repo").as_deref(),
        Some(r"C:\repo")
    );
}

#[test]
fn path_semantics_rejects_non_absolute_or_incomplete_paths() {
    for input in ["", "   ", "repo", "C:repo", r"\\server"] {
        assert_eq!(
            path::normalize(PathFlavor::Windows, input),
            None,
            "windows reject {input:?}"
        );
    }
    for input in ["", "   ", "repo", r"C:\repo"] {
        assert_eq!(
            path::normalize(PathFlavor::Unix, input),
            None,
            "unix reject {input:?}"
        );
    }
    assert_eq!(path::normalize(PathFlavor::Unix, "/repo\0"), None);
}

#[test]
fn path_semantics_flavor_comparers_match_platform_case_rules() {
    assert!(path::paths_equal(
        PathFlavor::Windows,
        r"C:\Repo",
        r"c:\repo"
    ));
    assert!(!path::paths_equal(PathFlavor::Unix, "/Repo", "/repo"));
    assert!(path::paths_equal(
        PathFlavor::Windows,
        r"C:\RÉSUMÉ",
        r"c:\résumé"
    ));
    assert!(!PathFlavor::Windows.case_sensitive());
    assert!(PathFlavor::Unix.case_sensitive());
}

#[cfg(windows)]
#[test]
fn path_semantics_native_windows_root_relative_path_becomes_absolute() {
    let normalized = path::normalize_native(r"\repo").expect("root-relative Windows path");
    assert!(normalized.ends_with(r":\repo"));
}

#[test]
fn path_semantics_compare_is_case_ruled_and_total() {
    use std::cmp::Ordering;

    assert_eq!(
        path::compare(PathFlavor::Windows, r"C:\Repo", r"C:\repo"),
        Ordering::Equal
    );
    assert_eq!(path::compare(PathFlavor::Unix, "/a", "/a"), Ordering::Equal);
    assert!(path::compare(PathFlavor::Unix, "/a", "/b").is_lt());
    assert!(path::compare(PathFlavor::Unix, "/b", "/a").is_gt());
}

#[test]
fn path_semantics_display_name_is_never_empty_for_a_valid_root() {
    let cases = [
        (PathFlavor::Windows, r"C:\", r"C:\"),
        (PathFlavor::Windows, r"\\server\share", r"\\server\share"),
        (PathFlavor::Windows, r"C:\repo", "repo"),
        (PathFlavor::Unix, "/", "/"),
        (PathFlavor::Unix, "/repo", "repo"),
    ];
    for (flavor, input, expected) in cases {
        assert_eq!(
            path::display_name(flavor, input).as_deref(),
            Some(expected),
            "display name {input:?}"
        );
    }
}

#[test]
fn path_semantics_display_name_is_none_for_invalid_input() {
    assert_eq!(path::display_name(PathFlavor::Windows, "relative"), None);
    assert_eq!(path::display_name(PathFlavor::Unix, "relative"), None);
}

#[test]
fn path_semantics_same_or_child_respects_component_boundaries() {
    assert!(path::is_same_or_child(
        PathFlavor::Windows,
        r"C:\Repo",
        r"C:\repo"
    ));
    assert!(path::is_same_or_child(
        PathFlavor::Windows,
        r"C:\repo\child",
        r"C:\repo"
    ));
    assert!(path::is_same_or_child(
        PathFlavor::Windows,
        r"C:\repo\child\grand",
        r"C:\repo\child"
    ));
    assert!(!path::is_same_or_child(
        PathFlavor::Windows,
        r"C:\repo2",
        r"C:\repo"
    ));
    assert!(!path::is_same_or_child(
        PathFlavor::Windows,
        r"C:\repo",
        r"C:\repo\child"
    ));

    assert!(path::is_same_or_child(
        PathFlavor::Unix,
        "/repo/child",
        "/repo"
    ));
    assert!(!path::is_same_or_child(PathFlavor::Unix, "/repo", "/Repo"));
    assert!(!path::is_same_or_child(PathFlavor::Unix, "/repo2", "/repo"));
    assert!(!path::is_same_or_child(
        PathFlavor::Unix,
        "/repo",
        "/repo/child"
    ));
}

#[test]
fn path_semantics_native_flavor_accepts_the_host_root() {
    let flavor = PathFlavor::native();
    let root = match flavor {
        PathFlavor::Windows => r"C:\",
        PathFlavor::Unix => "/",
    };
    assert_eq!(path::normalize(flavor, root).as_deref(), Some(root));
    assert!(path::paths_equal(flavor, root, root));
}

#[test]
fn path_semantics_native_normalize_rejects_relative_and_blank_input() {
    assert_eq!(path::normalize_native(""), None);
    assert_eq!(path::normalize_native("   "), None);
    assert_eq!(path::normalize_native("relative/path"), None);

    match PathFlavor::native() {
        PathFlavor::Unix => {
            // A Windows drive path is not a native absolute path on Unix.
            assert_eq!(path::normalize_native(r"C:\repo"), None);
            assert_eq!(
                path::normalize_native("/tmp/./x/../y").as_deref(),
                Some("/tmp/y")
            );
        }
        PathFlavor::Windows => {
            // A bare drive-relative path is not absolute on Windows either.
            assert_eq!(path::normalize_native("C:repo"), None);
            assert_eq!(
                path::normalize_native(r"C:\repo\.\child\..\").as_deref(),
                Some(r"C:\repo")
            );
        }
    }
}
