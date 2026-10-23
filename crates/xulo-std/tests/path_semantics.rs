//! The `Path` namespace semantics in isolation — pure string operations, no
//! filesystem touched. Expected values follow POSIX / Python's `posixpath`.

use xulo_std::path;

#[test]
fn join_appends_relative_and_honours_absolute_and_slashes() {
    assert_eq!(path::join("a", "b"), "a/b");
    assert_eq!(path::join("a/", "b"), "a/b");
    assert_eq!(path::join("", "b"), "b");
    assert_eq!(path::join("a", ""), "a/");
    // An absolute right operand replaces the left one (os.path.join rule).
    assert_eq!(path::join("a", "/b"), "/b");
    assert_eq!(path::join("/base/dir", "sub/file"), "/base/dir/sub/file");
    // `\` is an ordinary character under POSIX semantics.
    assert_eq!(path::join("a", "b\\c"), "a/b\\c");
}

#[test]
fn dirname_takes_the_parent_ignoring_trailing_slashes() {
    assert_eq!(path::dirname("/a/b/c"), "/a/b");
    assert_eq!(path::dirname("/a/b/"), "/a");
    assert_eq!(path::dirname("a/b"), "a");
    assert_eq!(path::dirname("/a"), "/");
    assert_eq!(path::dirname("a"), "");
    assert_eq!(path::dirname(""), "");
    assert_eq!(path::dirname("/"), "/");
    assert_eq!(path::dirname("///"), "/");
}

#[test]
fn basename_takes_the_final_component() {
    assert_eq!(path::basename("/a/b/c.txt"), "c.txt");
    assert_eq!(path::basename("/a/b/"), "b");
    assert_eq!(path::basename("file"), "file");
    assert_eq!(path::basename("/"), "");
    assert_eq!(path::basename(""), "");
}

#[test]
fn ext_takes_the_dotted_suffix_of_the_final_component() {
    assert_eq!(path::ext("report.pdf"), ".pdf");
    assert_eq!(path::ext("/a/b/archive.tar.gz"), ".gz");
    assert_eq!(path::ext("/a/b/noext"), "");
    // A leading dot names a hidden file; a trailing dot carries no text.
    assert_eq!(path::ext("/home/u/.bashrc"), "");
    assert_eq!(path::ext("file."), "");
    // Only the final component contributes.
    assert_eq!(path::ext("/a.b/file"), "");
    // Trailing slashes are ignored before the final component is taken.
    assert_eq!(path::ext("dir.d/"), ".d");
    assert_eq!(path::basename("dir.d/"), "dir.d");
}

#[test]
fn normalize_collapses_slashes_dots_and_dotdots() {
    assert_eq!(path::normalize("/a/./b//c/"), "/a/b/c");
    assert_eq!(path::normalize("a/b/../c"), "a/c");
    assert_eq!(path::normalize("/a/b/../../c"), "/c");
    // ".." above the root of an absolute path is dropped.
    assert_eq!(path::normalize("/.."), "/");
    assert_eq!(path::normalize("/../a"), "/a");
    // A relative path keeps unmatched ".." (directories may appear later).
    assert_eq!(path::normalize("../a"), "../a");
    assert_eq!(path::normalize("a/../../b"), "../b");
    // Nothing left: "." (or "/" for the root).
    assert_eq!(path::normalize(""), ".");
    assert_eq!(path::normalize("."), ".");
    assert_eq!(path::normalize("a/.."), ".");
    assert_eq!(path::normalize("/"), "/");
    // Trailing slashes are removed everywhere else.
    assert_eq!(path::normalize("a/b/"), "a/b");
}

#[test]
fn members_compose_for_the_usual_wiring() {
    let full = "/var/log/app/../app.log";
    let normalized = path::normalize(full);
    assert_eq!(normalized, "/var/log/app.log");
    let parent = path::dirname(&normalized);
    assert_eq!(parent, "/var/log");
    assert_eq!(path::basename(&normalized), "app.log");
    assert_eq!(path::ext(&normalized), ".log");
    assert_eq!(path::join(&parent, "rotate.conf"), "/var/log/rotate.conf");
}
