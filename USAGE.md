# stt-cli Usage

A practical guide to living with `stt-cli` day to day. For installation and the
project overview, see the [README](README.md).

## Installation

### Homebrew

```sh
gh auth login
gh auth setup-git
brew tap channprj/tap
brew install channprj/tap/stt-cli
```

The stable formula builds a release tag pinned to its exact Git commit. It
requires private-repository read access and authenticated Git; Homebrew installs
Rust as a build dependency. `--HEAD` is optional and selects unreleased `main`.

Use `stt-cli update` for stable and HEAD installations. Older versions can use
`brew update` followed by `brew upgrade channprj/tap/stt-cli` for stable releases
or `brew upgrade --fetch-HEAD channprj/tap/stt-cli` for HEAD installations.

### From source

```sh
git clone https://github.com/channprj/stt-cli.git
cd stt-cli
cargo install --path . --locked
```

Source builds require Rust 1.88 or newer. Use `cargo build --release` when you
want `target/release/stt-cli` without installing it.

### Optional tools

| Tool | Needed for |
|---|---|
| `brew` | `stt-cli update`, including migration from a standalone or Cargo installation |
| `gh` | Private repository authentication and `stt-cli update --check` |
| `ffmpeg` | `--vad` silence detection and compressed audio generation |
| `ffprobe` | Duration and cost information in dry-run output |
| `jq` | The JSON post-processing examples in this guide |

## The one idea

Transcription models report *offsets* — "this sentence starts 305 seconds in".
That is useless when you come back to a meeting recording three weeks later and
want to know what was said around half past two.

`stt-cli` closes the gap using something you already have: the start time your
recorder put in the file name. Offset plus start time equals wall-clock time.

```
20260815_143000_standup.m4a   →  starts 2026-08-15 14:30:00
segment at +305s              →  [2026-08-15 14:35:05]
```

Everything else in this guide follows from that.

## Quick start

```sh
stt-cli config set openai        # or: soniox / groq
stt-cli config show              # confirm the key landed
stt-cli transcribe some-recording.m4a
```

If you skip the key, `stt-cli` tells you exactly what to run rather than failing
with an HTTP error.

## Command reference

`stt-cli` requires one of three subcommands. `transcribe` and `update` also
have the visible aliases `tr` and `up`.

### `stt-cli transcribe <FILE>`

| Option | Meaning |
|---|---|
| `-p, --provider <NAME>` | `openai`, `soniox`, or `groq`; otherwise use the configured default or the first provider with a key |
| `-m, --model <MODEL>` | Override the provider's default model |
| `-l, --language <CODE>` | Supply a spoken-language hint such as `ko` or `en` |
| `-f, --format <FORMAT>` | `text` (default), `json`, `srt`, `vtt`, `txt`, or `csv` |
| `-o, --output <PATH>` | Write the rendered transcript to a file instead of stdout |
| `-s, --start <WHEN>` | Override the recording start time |
| `-n, --dry-run` | Validate and preview the request without calling a transcription API |
| `--vad` | Remove silence before upload and map returned offsets back to the original timeline |
| `--vad-threshold <DB>` | Silence threshold in dB; default `-35` |
| `--vad-min-silence <SECONDS>` | Minimum gap treated as silence; default `0.5` |

Provider defaults are `whisper-1` for OpenAI, `stt-async-v5` for Soniox, and
`whisper-large-v3` for Groq. OpenAI and Groq accept files up to 25 MB in this
client. Models whose names do not begin with `whisper` are treated as
non-timestamped by the OpenAI-compatible adapters and produce one segment when
the API returns only text.

### `stt-cli config <ACTION>`

| Command | Meaning |
|---|---|
| `config set <PROVIDER> [API_KEY]` | Store a key; omit the value to read it from stdin |
| `config unset <PROVIDER>` | Remove a stored key |
| `config show` | Show masked keys, their source, and the selected default |
| `config default <PROVIDER>` | Select the provider used when `--provider` is omitted |
| `config path` | Print the credential file path |

### `stt-cli update [--check]`

`stt-cli update` uses Homebrew on macOS and Linux. It runs `brew update`, ensures
`channprj/tap` is tapped, and chooses the action from the installed formula:

| Installation | Action |
|---|---|
| No Homebrew formula installed | `brew install channprj/tap/stt-cli` |
| Stable Homebrew installation | `brew upgrade channprj/tap/stt-cli` |
| Active HEAD installation | `brew upgrade --fetch-HEAD channprj/tap/stt-cli` |
| Standalone or Cargo binary alongside Homebrew | Update the formula, then migrate the running binary's path |

Homebrew must already be installed with `brew` on PATH. If it is missing, the
command explains how to install it and leaves the executable unchanged. Source
installs and upgrades still require authenticated Git access to the private
repository. See the [Homebrew command reference](https://docs.brew.sh/Manpage)
for the underlying commands.

For a standalone or Cargo binary, the command keeps the original at
`.stt-cli-update-<pid>/stt-cli` beside the executable. After Homebrew succeeds,
it verifies the installed executable and links the old path to Homebrew's
`opt/stt-cli/bin/stt-cli`. Existing PATH entries and aliases therefore continue
to work and follow later Homebrew upgrades. This migration runs even if the
standalone binary reports the same version as the stable formula. A new
Homebrew install selects the published stable release; an existing active HEAD
install stays on HEAD.

A standalone executable in Homebrew's own `bin` directory is backed up before
freeing its conflicting path. If the update or verification fails, the command
restores that original. Other standalone paths remain unchanged until the
Homebrew executable is verified. Successful migrations retain the backup and
print its location. The API-key configuration is preserved. Homebrew manages
its own Cellar files; the updater never replaces those files itself.

`stt-cli update --check` compares Headatever versions numerically with the
latest GitHub Release using `gh`. It does not run Homebrew, install software,
or migrate the current binary. A failed lookup is an error; equal or older
release tags report that no newer release is available. This checks published
releases, not unreleased commits on `main`.

## Configuration

The default path is `~/.config/stt-cli/api.json`; when `XDG_CONFIG_HOME` is
set, the file moves to `$XDG_CONFIG_HOME/stt-cli/api.json`. The directory is
restricted to mode `0700` and the file to `0600` on Unix.

Resolution is deliberately predictable:

1. `--provider` selects the provider for this run.
2. Otherwise `default_provider` from the config file is used.
3. Without a configured default, the first available key wins in OpenAI,
   Soniox, Groq order.
4. For the selected provider, its environment variable overrides the stored
   key: `OPENAI_API_KEY`, `SONIOX_API_KEY`, or `GROQ_API_KEY`.

Passing a key directly to `config set` works, but omitting it keeps the value
out of shell history. The stdin prompt is visible while typing, so avoid using
it where someone can watch the terminal.

## Name recordings so the timestamps work

This is the only thing worth changing about your habits. Most recorders already
do the right thing:

| Source | Name it produces | Works |
|---|---|---|
| iOS Voice Memos (renamed export) | `New Recording 2026-08-15 at 14.30.22.m4a` | yes |
| Android recorders | `20260815_143022.m4a` | yes |
| Zoom | `2026-08-15 14.30.22 Meeting.mp4` | yes |
| OBS / screen capture | `2026-08-15 14-30-22.mkv` | yes |
| Anything you rename yourself | `20260815143022-standup.wav` | yes |
| A bare title | `standup.m4a` | falls back to offsets |

When a recorder gives you something useless, rename on the way in:

```sh
mv memo.m4a "$(date -r memo.m4a +%Y%m%d_%H%M%S)_memo.m4a"
```

`date -r` reads the file's own modification time, which for a recording is
usually when it finished — close enough for most purposes, but pass `--start`
when you need it exact.

If you cannot rename, anchor the run instead:

```sh
stt-cli transcribe memo.m4a --start "2026-08-15 14:30"
```

`--start` accepts the same spellings as file names, so
`--start 20260815_1430` works just as well.

## Choosing a provider

| Situation | Use |
|---|---|
| File under 25 MB, want it done in one request | `-p openai` or `-p groq` |
| Long recording, or over 25 MB | `-p soniox` |
| You want per-token timing accuracy | `-p soniox` |
| Fastest turnaround | `-p groq` (LPU-accelerated Whisper) |
| Mixed or uncertain language | any, with `-l` omitted |

Set the one you reach for most as the default and stop typing `-p`:

```sh
stt-cli config default soniox
```

All three keys can live side by side; `--provider` overrides the default for
one run. Soniox timestamps individual tokens and preserves speaker labels when
the API supplies them. OpenAI and Groq use the same synchronous
OpenAI-compatible response shape.

## Choosing an output format

| Format | Best for | Time representation |
|---|---|---|
| `text` | Reading, searching, and concatenating transcripts | Wall-clock time when anchored; relative offset otherwise |
| `json` | Automation and later re-anchoring | Numeric offsets plus optional `started_at` and `at` |
| `srt` | Video editors and media players | Relative subtitle time with comma milliseconds |
| `vtt` | HTML5 `<track>` and web players | Relative subtitle time with dot milliseconds |
| `txt` | Clean copy and paste | No timestamps or speaker labels |
| `csv` | Spreadsheets and tabular analysis | Numeric offsets plus optional absolute time and speaker |

`text` and `json` expose the wall-clock feature directly. Subtitle formats
stay relative to the media so they remain synchronised regardless of the
recording date.

## Everyday recipes

### Transcript next to the audio

```sh
stt-cli transcribe 20260815_143000_standup.m4a -o 20260815_143000_standup.txt
```

### Straight into a file with a shell redirect

Progress lines go to stderr and the transcript to stdout, so redirecting gives
you a clean file while you still watch it work:

```sh
stt-cli transcribe recording.m4a > recording.txt
```

### A whole folder

`stt-cli` takes one file per run, which makes batching a plain shell loop:

```sh
for f in ~/Recordings/*.m4a; do
  stt-cli transcribe "$f" -o "${f%.*}.txt"
done
```

Add `|| continue` if you would rather skip failures than stop:

```sh
for f in ~/Recordings/*.m4a; do
  stt-cli transcribe "$f" -o "${f%.*}.txt" || continue
done
```

### Subtitles for video

Export SubRip for video editing / media players:

```sh
stt-cli transcribe lecture.m4a -f srt -o lecture.srt
```

WebVTT for HTML5 `<track>` or web-based players:

```sh
stt-cli transcribe meeting.m4a -f vtt -o meeting.vtt
```

Both subtitle formats use offset-based timestamps (not wall-clock), so they
remain synchronised with the media regardless of when the recording started.

### Plain text and CSV

Strip all timestamps for a clean copy-paste:

```sh
stt-cli transcribe notes.m4a -f txt > notes.txt
```

Export structured rows for spreadsheet analysis:

```sh
stt-cli transcribe data.m4a -f csv > data.csv
```

### Korean, or any specific language

```sh
stt-cli transcribe 20260815_143000_회의.m4a -l ko
```

The hint mainly helps short or noisy recordings. Leave it off for meetings that
switch between languages.

### Recordings over 25 MB on OpenAI or Groq

Either switch provider:

```sh
stt-cli transcribe long.m4a -p soniox
```

…or split, keeping the clock honest by naming each part with its own start time:

```sh
ffmpeg -i 20260815_140000_allhands.m4a -f segment -segment_time 900 -c copy part%03d.m4a
# part000 starts at 14:00, part001 at 14:15, part002 at 14:30, …
stt-cli transcribe part001.m4a --start "2026-08-15 14:15"
```

### Post-processing the JSON

```sh
stt-cli transcribe standup.m4a -f json -o standup.json
```

### Cost-optimised transcription with VAD

Trim silence to pay only for the speech segments:

```sh
stt-cli transcribe long_meeting.m4a --vad
```

Preview the saving first:

```sh
stt-cli transcribe long_meeting.m4a --dry-run --vad
```

Adjust the silence detector for your recording environment:

```sh
stt-cli transcribe noisy_room.m4a --vad --vad-threshold=-45 --vad-min-silence=1.0
```

What was said between 14:30 and 14:35?

```sh
jq -r '.segments[] | select(.at >= "2026-08-15T14:30:00" and .at < "2026-08-15T14:35:00") | "\(.at)  \(.text)"' standup.json
```

ISO-8601 strings sort correctly as plain text, so a string comparison is all a
time-window filter needs.

As CSV for a spreadsheet:

```sh
jq -r '.segments[] | [.at, .start, .text] | @csv' standup.json
```

Everything a single speaker said, when the provider labelled speakers:

```sh
jq -r '.segments[] | select(.speaker == "Speaker 1") | .text' standup.json
```

Append several meetings into one searchable daily log:

```sh
for f in ~/Recordings/20260815_*.m4a; do
  stt-cli transcribe "$f"
done >> ~/notes/2026-08-15.md
```

Because every line carries an absolute timestamp, the concatenated file stays in
chronological order and stays greppable:

```sh
grep '2026-08-15 14:3' ~/notes/2026-08-15.md
```

## Dry-run: preview before transcribing

Pass `--dry-run` (or `-n`) to validate the file and see what would be done
without calling any API:

```sh
stt-cli transcribe recording.m4a --dry-run
```

The output shows the file size, resolved provider and model, the start-time
anchor, the target format, and — crucially — the **estimated cost**:

```console
── dry run ──────────────────────────────
  file:      20260815_143000_standup.m4a
  size:      18.3MB
  provider:  groq
  model:     whisper-large-v3
  duration:  45:30 (18.3MB)
  est. cost: $0.0106
─────────────────────────────────────────
```

Add `--vad` to also see the speech-only estimate:

```sh
stt-cli transcribe recording.m4a --dry-run --vad
```

```console
── dry run ──────────────────────────────
  file:      20260815_143000_standup.m4a
  size:      18.3MB
  provider:  openai
  model:     whisper-1
  duration:  45:30 (18.3MB)
  speech:    25:10 (55% of audio)
  est. cost: $0.0042 (VAD saves $0.0034)
─────────────────────────────────────────
```

## Cutting cost with VAD (voice activity detection)

Transcription APIs bill by audio duration, so minutes of silence cost the same
as minutes of speech. `--vad` detects the actual speech segments with
`ffmpeg`'s `silencedetect`, trims the silence into a compressed copy, and
transcribes only that:

```sh
stt-cli transcribe 20260815_143000_allhands.m4a --vad
```

The wall-clock timestamps stay honest: after transcription the offsets are
mapped back onto the original timeline, so a line spoken at 14:35 is still
stamped 14:35 even though the silence between sentences was removed.

Two knobs tune the detector:

```sh
stt-cli transcribe long.m4a --vad \
  --vad-threshold=-35 \
  --vad-min-silence=0.5
```

The defaults are `-35` dB and `0.5` seconds. A higher threshold (for example,
`-30`) treats more quiet audio as silence and cuts more aggressively; a lower
one (`-50`) preserves near-silent speech. Music and noisy rooms benefit from
`-45` or lower.

Dry-run with `--vad` shows the speech ratio and the expected saving before you
spend anything.

> A real `--vad` transcription requires `ffmpeg` on `PATH`; `ffprobe` is
> used to determine the complete source duration and should be installed with
> it. The current implementation returns an error when `ffmpeg` cannot detect
> or build the compressed audio. During dry-run, a failed VAD estimate falls
> back to the full duration and does not call a transcription API.

## Reading the output

```
[2026-08-15 14:30:05] Speaker 1: Good morning.
 └── when it was spoken   └── only present if the provider labelled speakers
```

Two things to keep in mind:

- The timestamp is **inferred**, not measured. It is the file name's start time
  plus the model's offset. If the file name lies, the timestamps lie.
- Times are local wall-clock times with no time-zone attached. A recording made
  in another zone keeps whatever the file name said.

When no start time is available you get relative offsets instead, and a warning
that says so:

```
! no date or time in "meeting.m4a" — timestamps stay relative
  anchor them with --start "2026-08-15 14:30"
[00:00:05] Good morning.
```

## Where things live

| What | Where |
|---|---|
| API keys | `~/.config/stt-cli/api.json`, mode `0600` |
| Path, printed on demand | `stt-cli config path` |
| Overrides | `OPENAI_API_KEY`, `GROQ_API_KEY`, `SONIOX_API_KEY` |

Outside an explicit `--output` path, no persistent cache, history, or log is
created. VAD cleans its temporary working files. A successful Homebrew
migration retains the original binary in `.stt-cli-update-<pid>/stt-cli` beside
the former executable; the command prints the backup location. After a Soniox
transcription job is created, the client attempts to delete both the job and uploaded file after
polling; a failure before job creation can leave the uploaded file behind.
Remote retention for OpenAI and Groq is governed by the corresponding account
and provider policies.

## Troubleshooting

| Message | What it means |
|---|---|
| `no API key for openai.` | No key stored or exported. The message lists both ways to fix it. |
| `… is not a readable file` | Wrong path, or a directory was passed. |
| `cannot read a date and time from "yesterday"` | `--start` needs a real date, e.g. `"2026-08-15 14:30"`. |
| `… is 30.0 MB — OpenAI accepts at most 25 MB` | Use `-p soniox`, or split with ffmpeg. |
| `openai transcription failed (401)` | The key is wrong or revoked. |
| `groq transcription failed (401)` | The `GROQ_API_KEY` is wrong or revoked. |
| `soniox did not finish within 30 minutes` | The job is stuck; retry, or split the file. |
| `cannot run ffmpeg — is it installed and on PATH?` | `--vad` needs `ffmpeg`; install it or run without VAD. |
| `duration: unknown` in dry-run | `ffprobe` is missing or cannot read the media. Validation can continue, but no cost estimate is shown. |
| `cannot run brew` | Install Homebrew from https://brew.sh and ensure `brew` is on PATH, then retry `stt-cli update`. |
| `cannot check GitHub releases` / `cannot run gh release view` | `update --check` needs `gh`, authentication, and a published GitHub Release. |
| A `brew` command failed during `update` | Resolve the displayed Homebrew or Git authentication error and retry. Standalone executables are preserved or restored on failure. |
| `! no speech was recognised` | Silence, an unsupported codec, or the wrong `-l` hint. |
| `! no date or time in "…"` | Expected for un-dated names — pass `--start` if you need absolute times. |

Colours disappear when you pipe output; that is deliberate, and `NO_COLOR=1`
turns them off in a terminal too.

## Keeping it current

Use the same command for Homebrew updates and migration from other installation
methods. Stable Homebrew installs track published Headatever tags:

```sh
stt-cli update
brew list --versions stt-cli
stt-cli --version
```

For an intentional development install (`brew install --HEAD
channprj/tap/stt-cli`), `stt-cli update` uses `brew upgrade --fetch-HEAD
channprj/tap/stt-cli` to check and update `main`. An existing HEAD installation
can be switched to stable by uninstalling and then installing without `--HEAD`;
this does not remove the API-key config.

```sh
brew uninstall stt-cli
brew install channprj/tap/stt-cli
```

To keep a Cargo installation instead of migrating it to Homebrew, update the
source checkout and run `cargo install --path . --locked`. Use
`stt-cli update --check` when you only want to check published releases.
