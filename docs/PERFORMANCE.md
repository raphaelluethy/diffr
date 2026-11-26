# Performance tests

Run `scripts/perf.sh --offline` from a checkout with the locked dependencies cached. Omit `--offline` if Cargo needs to fetch them. The script builds with the workspace's dev optimizations, disables debug information and incremental compilation by default, and runs the benchmarks serially. It removes its temporary target directory on exit. Set `CARGO_TARGET_DIR` to retain and reuse a build cache.

The diff fixture has 12 files with 4,000 replacement pairs each. It measures word highlighting for a first viewport of 60 rows. The review fixture has 2,000 changed files and measures local prompt preparation and JSON answer parsing with absolute file paths. Each benchmark discards two warm-up samples and reports the median and p95 of 20 samples. The timings cover local operations. GPU rendering and provider response times are outside these measurements.

## Linux results

Measured on an AMD Ryzen 5 6600H with Cargo 1.97.1. The baseline was `6f31c40` with the same fixtures and the previous eager word highlighting path.

| Operation | Baseline median | Updated median | Baseline p95 | Updated p95 |
| --- | ---: | ---: | ---: | ---: |
| First viewport word highlighting | 75.827 ms | 0.073 ms | 115.280 ms | 0.084 ms |
| Review prompt preparation | 0.824 ms | 0.354 ms | 0.897 ms | 0.368 ms |
| Review answer parsing and path matching | 341.349 ms | 1.829 ms | 345.740 ms | 1.872 ms |

The diff view computes word marks as rows enter the viewport and reuses them when revisiting those rows. It shares immutable file snapshots with background content loading and review preparation. The desktop smoke test opened and scrolled a synthetic Git working tree with 48,000 additions and 48,000 deletions under Xvfb.

The structured review instructions sent to Claude Code and Codex went from 6,929 to 3,578 bytes in the default configuration. Both still receive the complete schema through their structured-output option. The agent tests use local mock CLIs to exercise inline patches, temporary patches, structured responses, fenced responses and cancellation.

## Blaze

Blaze v0.20261122.0 started with Linux io_uring access and stopped because its pinned toolchain manifest was missing. The release's bootstrap command did not support fetching it. [Release](https://github.com/indexable-inc/blaze/releases/tag/v0.20261122.0)

Blaze v0.20261030.1 checked the `diffr-agent` package in 17.42 seconds on its first scoped run following the full-app trial. Its full-app check stopped after 181 seconds while loading `const-random-macro v0.1.16`, which was missing from its WebAssembly proc-macro graph. [Release](https://github.com/indexable-inc/blaze/releases/tag/v0.20261030.1)

The warm comparison ran in a temporary checkout with registry downloads disabled. Both tools used four build jobs, and the first sample in each series was discarded. The executable probe called the review-agent prompt formatter. Each edit changed a string inside that function, and each rebuilt probe was run to verify the changed output.

| Scoped review-agent operation | Cargo 1.97.1 median | Blaze v0.20261030.1 median |
| --- | ---: | ---: |
| Check with no changes | 0.15 s | 0.04 s |
| Check after a function-body edit | 0.40 s | 0.40 s |
| Build the executable probe with no changes | 0.16 s | 0.01 s |
| Rebuild the executable probe after a function-body edit | 1.00 s | 0.08 s |

Blaze rebuilt the probe about 12.5 times as fast after an edit. The full-app compatibility error prevented a comparable app build. The trials used about 15 GiB of temporary artifacts at peak, including both build stores and the pinned nightly. Those artifacts were removed after verification.
