# stt-cli

Transcribe an audio file and stamp every line with the wall-clock time it was
spoken — not the offset into the recording.

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

## Install

```sh
cargo install --path .
```

## Register an API key

Either provider works; pick whichever you have an account for.

```sh
stt-cli config set openai      # paste the key at the prompt
stt-cli config set soniox
stt-cli config default soniox  # used when --provider is omitted
stt-cli config show
```

Keys are written to `~/.config/stt-cli/api.json` (honouring `XDG_CONFIG_HOME`)
with `0600` permissions. Passing the key as an argument works too, but the
prompt keeps it out of your shell history.

`OPENAI_API_KEY` and `SONIOX_API_KEY` take precedence over the stored keys, so
a one-off run needs no configuration at all.

## Usage

```sh
stt-cli transcribe recording.m4a                         # timestamped text on stdout
stt-cli transcribe recording.m4a -f json -o out.json     # machine-readable
stt-cli transcribe recording.m4a -l ko                   # language hint
stt-cli transcribe recording.m4a -p soniox               # choose the backend
stt-cli transcribe rec.m4a --start "2026-08-15 14:30"    # anchor it yourself
```

Run `stt-cli` with no arguments for the full help.

### Start times read from file names

Any of these yield `2026-08-15 14:30:22`:

| File name |
|---|
| `20260815_143022.m4a` |
| `20260815143022.wav` |
| `2026-08-15_14-30-22.mp3` |
| `2026-08-15 14.30.22.m4a` |
| `2026-08-15T14:30:22.flac` |
| `New Recording 2026-08-15 at 14.30.22.m4a` |
| `IMG_20260815_143022.mov` |

Seconds are optional (`zoom_20260815_1430.mp4`). A date with no time is anchored
at midnight (`notes-2026-08-15.wav` → `2026-08-15 00:00:00`).

When nothing date-like is found, `stt-cli` says so and falls back to relative
`[00:00:05]` offsets rather than inventing a time. `--start` accepts the same
spellings and always wins.

Times are treated as local wall-clock times; no time-zone conversion happens.

### Output

`-f text` (default) is one line per utterance:

```
[2026-08-15 14:30:05] Good morning.
```

`-f json` keeps both views, so you can re-anchor or post-process later:

```json
{
  "file": "20260815_143000_standup.m4a",
  "provider": "openai",
  "model": "whisper-1",
  "started_at": "2026-08-15T14:30:00",
  "segments": [
    { "start": 5.0, "end": 8.2, "at": "2026-08-15T14:30:05", "text": "Good morning." }
  ]
}
```

`started_at` and `at` are omitted entirely when no start time is known.

## Providers

| | OpenAI | Soniox |
|---|---|---|
| Default model | `whisper-1` | `stt-async-v5` |
| Timestamps | per segment | per token, grouped into utterances |
| Upload limit | 25 MB | none in practice |
| How it runs | one request | upload, poll, fetch |

Override the model with `-m`. On OpenAI only the `whisper-*` models return
timings — the `gpt-4o-transcribe` family returns text alone, and `stt-cli` warns
and emits a single segment if you ask for one.

Files sent to Soniox are deleted from Soniox again once the transcript has been
fetched.

Beyond 25 MB on OpenAI, either switch provider or split the file:

```sh
ffmpeg -i long.m4a -f segment -segment_time 900 -c copy part%03d.m4a
```

Naming the parts so each one carries its own start time keeps the timestamps
honest across the split.

## Development

```sh
cargo test
cargo clippy --all-targets
```
