# Setting up diffr

diffr is built from source with Cargo. This guide covers what it needs, how
to build and install it, and how to connect GitHub and an AI agent.

diffr is published as it is, without support: see
[Support and contributions](../README.md#support-and-contributions).

- [Requirements](#requirements)
- [Build and run](#build-and-run)
- [Install](#install)
- [GitHub pull requests](#github-pull-requests)
- [AI review](#ai-review)
- [Your editor](#your-editor)
- [Files diffr writes](#files-diffr-writes)
- [Environment variables](#environment-variables)
- [Troubleshooting](#troubleshooting)
- [Uninstall](#uninstall)

## Requirements

| | |
|---|---|
| **Git** | on your `PATH`; diffr reads and writes repositories through the `git` command |
| **Rust** | the current stable toolchain, from [rustup](https://rustup.rs) (the workspace uses edition 2024) |
| **macOS** | 11 or later, with Xcode and its Metal Toolchain |
| **Linux** | X11 or Wayland, a Vulkan driver, and the development packages below |

Windows is not tested.

### macOS

GPUI compiles its Metal shaders at build time, which takes the full Xcode,
not the command line tools alone:

```sh
xcode-select --install
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
xcodebuild -downloadComponent MetalToolchain
```

### Linux

The build needs a C toolchain, `pkg-config`, libclang (for bindgen), and
the headers of fontconfig, FreeType, Wayland, xkbcommon and X11/XCB. To run,
diffr needs a Vulkan driver. Package names vary by distribution; these are a
starting point.

Debian and Ubuntu:

```sh
sudo apt install build-essential clang pkg-config \
  libfontconfig-dev libfreetype-dev libwayland-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev \
  libvulkan1 mesa-vulkan-drivers
```

Fedora:

```sh
sudo dnf install gcc gcc-c++ clang pkgconf-pkg-config \
  fontconfig-devel freetype-devel wayland-devel \
  libxkbcommon-devel libxkbcommon-x11-devel libxcb-devel libX11-devel \
  vulkan-loader mesa-vulkan-drivers
```

Optional: `notify-send` (libnotify) for the notification when a background
AI review finishes, and `xdg-mime` (xdg-utils) for `diffr://` links.

## Build and run

```sh
git clone https://github.com/raphaelluethy/diffr.git
cd diffr
cargo run --release -p diffr -- /path/to/a/repository
```

The first build compiles GPUI and some sixty tree-sitter grammars and takes
several minutes; later builds are quick. The binary is
`target/release/diffr`. Without an argument it reviews the repository of
the current directory, or shows a welcome window with the repositories
opened lately. See [Usage](../README.md#usage) for commits, branches and
pull requests.

To check the build: `cargo test --workspace`.

## Install

### macOS: the app bundle

```sh
brew install librsvg                 # rsvg-convert, for the icon
cargo install --locked cargo-about   # for THIRD_PARTY_LICENSES.html
scripts/bundle.sh --install          # builds target/release/diffr.app, copies it to /Applications
```

The bundle is signed ad hoc: it runs on the machine that built it, and is
not notarized for distribution to others.

### Linux, or without a bundle

```sh
cargo install --locked --path crates/app   # ~/.cargo/bin/diffr
```

### The `diffr` command

In the app, choose **Install Command Line Tool…** (the application menu, the
welcome window or the command bar). It writes a small `diffr` launcher into
`/usr/local/bin`, or `~/.local/bin` when that is not writable, which
opens the app on the repository you run it in and returns at once. On Linux
it also adds a desktop entry, so diffr shows in the launcher and opens
`diffr://` links.

## GitHub pull requests

Listing and reviewing pull requests needs a token for github.com. diffr
takes the first of:

1. `githubToken` in the settings (Settings › GitHub);
2. the environment: `DIFFR_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`;
3. the GitHub CLI's login: run `gh auth login` once.

The GitHub CLI is the simplest: nothing is stored by diffr. A token of your
own must be able to read the repository and write pull request reviews.
Without a token, everything but pull requests works.

## AI review

diffr has no AI account or API key of its own. It runs a coding agent
already installed and signed in on your machine, read-only, and only when
you ask for a review or a commit message:

| Agent | Command diffr looks for |
|---|---|
| Claude Code | `claude` |
| Codex | `codex` |
| OpenCode | `opencode` |

Install one, sign in with it once in a terminal, and diffr finds it: it
looks in the `PATH` of your login shell, so agents installed with
Homebrew, npm or mise are found also when diffr is opened from the Dock or a
launcher. With several installed, diffr prefers them in the order above;
choose one, and its model, in Settings › AI (`aiAgent`, `aiModel`).

The changes under review are sent to the provider of the agent you chose,
under your account with it.

## Your editor

**Open the file in your editor** uses, in order: `$DIFFR_EDITOR`, the
`editorCommand` setting, Zed, VS Code, then the system's opener. A command
may contain `{file}`, `{line}` and `{repo}`:

```jsonc
"editorCommand": "code --goto {file}:{line}"
```

## Files diffr writes

| | |
|---|---|
| `~/.diffr/diffr.jsonc` | your settings; see [Configuration](../README.md#configuration) |
| `diffr/state.json` in the data directory (`~/Library/Application Support` on macOS, `~/.local/share` on Linux) | window places, recent repositories, viewed files |
| `/usr/local/bin/diffr` or `~/.local/bin/diffr` | the launcher, if you installed it |
| `~/.local/share/applications/dev.diffr.desktop` | the desktop entry (Linux), with the launcher |

In your repositories diffr changes only what you ask for: staging, commits,
pushes, and the fetch of a pull request's commits.

## Environment variables

| | |
|---|---|
| `DIFFR_GITHUB_TOKEN` | a GitHub token, before `GH_TOKEN` and `GITHUB_TOKEN` |
| `DIFFR_EDITOR` | the editor command, before the `editorCommand` setting |
| `DIFFR_THEME` | `dark` or `light`, over the `theme` setting |
| `DIFFR_NO_SHELL_PATH` | set to keep diffr from asking your login shell for its `PATH` |

## Troubleshooting

- **The build fails in `gpui` with a `metal` error (macOS).** The Metal
  Toolchain is missing, or `xcode-select` points at the command line
  tools: see [macOS](#macos).
- **The build fails on a missing `.pc` file or `libclang` (Linux).** A
  development package is missing: see [Linux](#linux).
- **No window, or an error about adapters (Linux).** No Vulkan driver was
  found. Install your GPU's, or `mesa-vulkan-drivers`; `vulkaninfo`
  shows what there is.
- **"No coding agent was found."** None of `claude`, `codex` and
  `opencode` is on the `PATH` of your login shell. Check with
  `$SHELL -lc 'command -v claude codex opencode'`.
- **Pull requests do not list.** Check `gh auth status`, and that the
  repository's remote is on github.com.
- **No notification when a review finishes.** On macOS notifications come
  from the app bundle (allow them in System Settings › Notifications); from
  `cargo run` they come through Script Editor. On Linux install
  `notify-send`.

## Uninstall

```sh
rm -rf /Applications/diffr.app ~/.diffr           # the app and your settings
rm -f /usr/local/bin/diffr ~/.local/bin/diffr     # the launcher
cargo uninstall diffr                             # if installed with cargo
rm -rf ~/Library/Application\ Support/diffr       # state (macOS)
rm -rf ~/.local/share/diffr ~/.local/share/applications/dev.diffr.desktop   # state and desktop entry (Linux)
```
