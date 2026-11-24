//! The order of files as a file tree lists them.

use crate::diff::File;
use std::cmp::Ordering;

/// Sorts files as a file tree lists them: at each level, directories before
/// files, each by name. The sort is stable.
pub fn sort_files(files: &mut [File]) {
    files.sort_by(|a, b| compare_paths(&a.path, &b.path));
}

/// Orders two paths as a file tree lists them.
pub fn compare_paths(a: &str, b: &str) -> Ordering {
    let as_: Vec<&str> = a.split('/').collect();
    let bs: Vec<&str> = b.split('/').collect();
    for i in 0..as_.len().min(bs.len()) {
        let a_dir = i < as_.len() - 1;
        let b_dir = i < bs.len() - 1;
        if a_dir != b_dir {
            return if a_dir {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let c = compare_names(as_[i], bs[i]);
        if c != Ordering::Equal {
            return c;
        }
    }
    as_.len().cmp(&bs.len())
}

fn compare_names(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order() {
        let mut paths = vec!["b.txt", "src/z.go", "A.txt", "src/a/b.go", "a.txt"];
        paths.sort_by(|a, b| compare_paths(a, b));
        assert_eq!(
            paths,
            vec!["src/a/b.go", "src/z.go", "A.txt", "a.txt", "b.txt"]
        );
    }
}
