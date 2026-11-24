//! Prints the fingerprints of a work tree's changed files as JSON, for
//! test fixtures: `cargo run -p diffr-git --example fingerprints -- <repo>`.
fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let repo = diffr_git::Repo::open(&dir).expect("a repository");
    let files = repo.working_tree(diffr_git::Options::default()).expect("changes");
    let map: std::collections::BTreeMap<String, String> =
        files.into_iter().map(|f| (f.path, f.fingerprint)).collect();
    println!("{}", serde_json::to_string(&map).unwrap());
}
