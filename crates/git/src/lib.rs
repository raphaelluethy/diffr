//! diffr's model of changes and its access to git, with the export of
//! review comments as Markdown and the settings file.
//!
//! - [`diff`]: changed files, hunks and lines, the parser of `git diff`
//!   output, and word-level changes within lines.
//! - [`git`]: [`Repo`], which runs the `git` binary to read the work tree,
//!   commits, ranges and file contents, and to commit.
//! - [`comments`]: review comments and their Markdown export.
//! - [`config`]: the user's settings (`~/.diffr/diffr.jsonc`) and the
//!   remembered state (viewed files, recent repositories).

pub mod comments;
pub mod config;
pub mod diff;
pub mod git;

pub use comments::{Comment, CommentSource, Comments, FileContents, Side, comments_markdown};
pub use config::{Config, Settings, Store};
pub use diff::{File, Hunk, Line, LineKind, Status};
pub use git::{Commit, Contents, Error, Options, Remote, Repo, Staged, Upstream};
