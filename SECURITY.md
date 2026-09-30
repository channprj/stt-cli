# Security

Security fixes are maintained on `main`. Published tags and downloaded binaries
are immutable snapshots: source fixes require a new release or a rebuild to
reach installed copies. The supported runtime is macOS/Linux with Rust 1.88+
for source builds.

## Credentials and recordings

- Omit the positional key in `stt-cli config set <provider>` to use hidden
  terminal input, or pipe it through stdin. Command-line keys can enter shell
  history and process listings.
- Configuration remains plaintext, protected by `0700` directory and `0600`
  file modes. Saves are atomic. Linked/special credential files and a symlinked
  application config directory are rejected. Use a trusted, absolute
  `XDG_CONFIG_HOME`; the parent directories and the current OS account must be
  trusted. This does not protect against root or a compromised login session.
- VAD uses a unique private temporary directory and cleans it on success and
  ordinary errors. A killed process or machine failure can leave private files.
  Old versions' shared `stt-cli-vad` directories are not automatically deleted;
  inspect and remove your own leftovers when no old process is running.
- Audio goes to the selected provider over HTTPS. Redirects are
  rejected. Soniox deletion is attempted for every known file/job even after
  errors. Warnings identify failed cleanup; network loss or termination can
  still require manual account-side deletion. Other providers' retention is
  governed by their account policies.
- API diagnostics redact the active key and do not dump malformed responses.
  Invalid configuration values are omitted from parsing/provider errors.
- Deliberate transcript exports can contain sensitive speech. `--output` writes
  an atomic `0600` file, including when replacing an existing ordinary file.
  Symbolic links, hard links and special files are rejected. Choose a trusted
  output directory. Shell redirects use the shell's permissions; set `umask 077`
  before redirecting sensitive transcripts.
- Transcript control characters, including terminal escape sequences and bidi
  overrides, are shown as literal escapes when stdout is a terminal. Files and
  pipes preserve the original data. Treat those exports as untrusted text when
  displaying them with other tools.
- CSV strings resembling formulas receive a quoted apostrophe prefix. Use JSON
  for exact data, and import untrusted spreadsheet columns as text. Spreadsheet
  software may remove protective prefixes when re-exporting a CSV.
- `ffmpeg`, `ffprobe`, `brew`, and `gh` are trusted programs resolved from PATH.
  Media protocols are restricted to local files; this is not an OS sandbox for
  the media decoder. Keep those external tools patched.

## Checks

```sh
gitleaks git --log-opts="--all --full-history" --redact=100 --ignore-gitleaks-allow --no-banner .
git log --all --format='%B' | gitleaks stdin --redact=100 --no-banner
gitleaks dir --redact=100 --ignore-gitleaks-allow --no-banner .
cargo audit --deny warnings
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
python3 -m unittest discover -s scripts -p 'test_*.py'
```

There are no GitHub Actions workflows in this repository. Run these checks
locally before publishing changes and releases, on both macOS and Linux, with
stable Rust and the minimum supported Rust 1.88. Install Gitleaks and
cargo-audit from their official distributions and keep their rules/advisory
DB current. Dependabot proposes Cargo updates weekly; it does not run these
checks or publish releases.

The published `v1.260906.0` release predates the security fixes on `main`.
Build current source or use Homebrew HEAD until a patched release and tap
formula are published. Version checks alone cannot distinguish an old release
binary from a newer source build with the same version. The old downloadable
binary also contains build-machine paths. New assets built with
`scripts/build-release.sh` remap home, Cargo-cache and project paths, and include
the MIT license notice with checksums. Verify the generated files before release.

## Reporting and response

Report vulnerabilities privately to the repository owner through an existing
trusted contact channel, or GitHub private vulnerability reporting if enabled.
Provide affected versions, reproduction steps, and redacted evidence. Never
paste live keys or private recordings into issues, logs, or pull requests.

If a credential is exposed, revoke/rotate it at its provider first. Removing a
file in a new commit does not remove historical copies. Coordinate any history
rewrite and remote cache/fork cleanup separately; ordinary pushes do not purge
those copies.
