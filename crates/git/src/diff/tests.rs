use super::*;

const SAMPLE_PATCH: &str = r#"diff --git a/main.go b/main.go
index 83db48f..bf2a3c1 100644
--- a/main.go
+++ b/main.go
@@ -1,5 +1,6 @@ package main
 package main

-import "fmt"
+import (
+	"fmt"
+)

 func main() {
@@ -10,3 +11,3 @@ func main() {
 	a := 1
-	fmt.Println(a)
+	fmt.Println(a + 1)
 }
\ No newline at end of file
diff --git a/old name.txt b/new name.txt
similarity index 90%
rename from old name.txt
rename to new name.txt
index 1111111..2222222 100644
--- a/old name.txt
+++ b/new name.txt
@@ -1 +1 @@
-hello
+hello world
diff --git a/img.png b/img.png
new file mode 100644
index 0000000..3333333
Binary files /dev/null and b/img.png differ
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index 4444444..0000000
--- a/gone.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-a
-b
"#;

#[test]
fn test_parse() {
    let files = parse(SAMPLE_PATCH.as_bytes());
    assert_eq!(files.len(), 4, "got {} files", files.len());
    let m = &files[0];
    assert!(
        m.path == "main.go" && m.status == Status::Modified && m.hunks.len() == 2,
        "main.go: {m:?}"
    );
    assert!(
        m.additions == 4 && m.deletions == 2,
        "main.go counts +{} -{}",
        m.additions,
        m.deletions
    );
    let h = &m.hunks[1];
    assert!(
        h.lines[0].old == 10 && h.lines[0].new == 11 && h.lines[3].no_newline,
        "second hunk lines: {:?}",
        h.lines
    );
    assert_eq!(h.section, "func main() {");
    let r = &files[1];
    assert!(
        r.status == Status::Renamed && r.old_path == "old name.txt" && r.path == "new name.txt",
        "rename: {r:?}"
    );
    let b = &files[2];
    assert!(
        b.binary && b.status == Status::Added && b.path == "img.png",
        "binary: {b:?}"
    );
    let d = &files[3];
    assert!(
        d.status == Status::Deleted && d.path == "gone.txt" && d.deletions == 2,
        "deleted: {d:?}"
    );
}

#[test]
fn test_word_diff() {
    let (ra, rb) = word_diff("\tfmt.Println(a)", "\tfmt.Println(a + 1)");
    assert!(ra.is_empty() && rb.len() == 1, "ranges {ra:?} {rb:?}");
    assert_eq!(&"\tfmt.Println(a + 1)"[rb[0].start..rb[0].end], " + 1");
    let (ra, rb) = word_diff("completely different", "nothing alike here at all");
    assert!(
        ra.is_empty() && rb.is_empty(),
        "rewritten lines marked: {ra:?} {rb:?}"
    );
}

#[test]
fn test_pairs() {
    let k = |kind| Line {
        kind,
        ..Line::default()
    };
    let lines = [
        k(LineKind::Context),
        k(LineKind::Del),
        k(LineKind::Del),
        k(LineKind::Add),
        k(LineKind::Context),
        k(LineKind::Add),
    ];
    let p = pairs(&lines);
    assert_eq!(p, vec![Pair { del: 1, add: 3 }]);
}

// Further tests, of details.

#[test]
fn test_quoted_paths_and_crlf() {
    let patch = b"diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\r\nindex 1..2 100644\r\n--- \"a/caf\\303\\251.txt\"\r\n+++ \"b/caf\\303\\251.txt\"\r\n@@ -1 +1 @@\r\n-a\r\r\n+b\r\r\n";
    let files = parse(patch);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "café.txt");
    assert_eq!(files[0].old_path, "café.txt");
    let l = &files[0].hunks[0].lines;
    assert_eq!(l[0].text, "a");
    assert_eq!(l[1].text, "b");
}

#[test]
fn test_new_file_from_content() {
    let f = new_file_from_content("x.txt", b"a\nb", Status::Untracked);
    assert_eq!(f.additions, 2);
    assert!(f.hunks[0].lines[1].no_newline);
    assert_eq!(f.fingerprint.len(), 16);
    let b = new_file_from_content("x.bin", b"a\0b", Status::Untracked);
    assert!(b.binary && b.hunks.is_empty());
    assert_eq!(split_lines("a\r\nb\n"), vec!["a", "b"]);
    assert!(split_lines("").is_empty());
}

#[test]
fn test_name_dir_gap() {
    let f = File {
        path: "src/a/b.rs".into(),
        hunks: vec![Hunk {
            old_start: 10,
            old_lines: 3,
            new_start: 11,
            new_lines: 3,
            ..Hunk::default()
        }],
        ..File::default()
    };
    assert_eq!(f.name(), "b.rs");
    assert_eq!(f.dir(), "src/a/");
    let g = f.gap_before(0, None, None);
    assert_eq!((g.old_start, g.new_start, g.count), (1, 1, 9));
    let g = f.gap_before(1, Some(20), Some(21));
    assert_eq!((g.old_start, g.new_start, g.count), (13, 14, 8));
}
