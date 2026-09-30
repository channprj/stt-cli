# stt-cli

**English** | [한국어](README.ko.md)

> Transcribe an audio file and stamp every line with the wall-clock time it was
> spoken — not the offset into the recording.

Recorders already write the start time into the file name. `stt-cli` reads it,
adds the offset the transcription model reports, and prints the result:

```console
$ stt-cli transcribe 20260815_143000_standup.m4a
→ recording starts 2026-08-15 14:30:00 (from the file name)
→ uploading to OpenAI (whisper-1)
[2026-08-15 14:30:05] 좋은 아침입니다. 어제 배포는 무사히 끝났습니다.
[2026-08-15 14:30:19] 오늘은 인덱싱 쪽을 보겠습니다.
```

The utterance five seconds in is reported at `14:30:05`, because the recording
began at `14:30:00`.

## Highlights

- Recovers local wall-clock start times from common recorder file names or
  accepts an explicit `--start` value.
- Uses OpenAI, Soniox, or Groq, with per-run provider and model overrides.
- Renders `text`, `json`, `srt`, `vtt`, `txt`, or `csv` output.
- Previews the resolved request and estimated cost with `--dry-run`.
- Optionally removes silence with `--vad` while mapping results back to the
  original recording timeline.
- Stores API keys in a permission-restricted config file and lets environment
  variables override them.
- Updates through Homebrew with `stt-cli update`, including migration from
  standalone and Cargo installations.

## Installation

### Homebrew

```sh
gh auth login
gh auth setup-git
brew tap channprj/tap
brew install channprj/tap/stt-cli
```

The stable formula builds a release tag pinned to an exact Git commit. Rust is
installed as a build dependency; `--HEAD` is not required. This repository is
private, so GitHub authentication and read access to `channprj/stt-cli` are
required. Existing authenticated Git credentials can be reused.

Update with:

```sh
stt-cli update
```

This refreshes Homebrew and installs or upgrades `channprj/tap/stt-cli`. It also
migrates standalone and Cargo binaries: the original is backed up beside the
executable, and the old path is linked to Homebrew so it keeps working on PATH.
Homebrew must be installed and `brew` must be on PATH; private-source Git access
is still required. `stt-cli update --check` only checks GitHub Releases using `gh`.

Older versions that ask you to update manually can use `brew update` followed by
`brew upgrade channprj/tap/stt-cli`.

For unreleased `main`, use `brew install --HEAD channprj/tap/stt-cli`.
`stt-cli update` preserves an active HEAD installation and checks upstream with
`brew upgrade --fetch-HEAD channprj/tap/stt-cli`.
Install `ffmpeg` separately if you want `--vad` or duration estimates.

### From source

```sh
git clone https://github.com/channprj/stt-cli.git
cd stt-cli
cargo install --path . --locked
```

Source builds require Rust 1.88 or newer. `cargo build --release` leaves the
binary at `target/release/stt-cli` without installing it.

## Quick start

Register one provider key, then transcribe a file:

```sh
stt-cli config set openai
stt-cli config show
stt-cli transcribe 20260815_143000_standup.m4a
```

You can register `soniox` or `groq` instead. `OPENAI_API_KEY`, `SONIOX_API_KEY`,
and `GROQ_API_KEY` override stored keys for one-off or automated runs.

```sh
stt-cli transcribe meeting.m4a --start "2026-08-15 14:30" -l ko
stt-cli transcribe meeting.m4a -p groq -f srt -o meeting.srt
stt-cli transcribe meeting.m4a --dry-run --vad
```

Run `stt-cli --help` or a subcommand with `--help` for the generated CLI
reference.

## More documentation

- [Usage](USAGE.md) — complete commands, configuration, output formats,
  workflows, and troubleshooting.
- [Architecture](ARCHITECTURE.md) — components, data flow, design decisions,
  current status, and extension guidance.
- [Security](SECURITY.md) — credential and audio handling, reporting, and audit commands.
- [Security audit, 2026-09-30](docs/security-audit-2026-09-30.md) — findings and verification (Korean).

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
python3 scripts/version.py check
python3 -m unittest discover -s scripts -p 'test_*.py'
```

See [Architecture](ARCHITECTURE.md#development-workflow) before adding a
provider, output format, timestamp pattern, or release path.

Versions follow [Headatever](https://github.com/channprj/headatever):
`head.yymmdd.patch`. `scripts/headatever.sh patch` synchronizes `VERSION`,
`Cargo.toml`, and `Cargo.lock`, commits them, and creates an annotated tag.
Release tooling requires Python 3.11 or newer. See the
[release workflow](ARCHITECTURE.md#release-checklist) before publishing.

## License

This repository does not currently include a `LICENSE` file. Confirm and add
the intended license before distributing the project outside its current
private scope.
