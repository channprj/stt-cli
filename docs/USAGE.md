# Using stt-cli locally

A practical guide to living with `stt-cli` day to day. For installation and the
option reference, see the [README](../README.md).

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

## First run

```sh
stt-cli config set openai        # or: stt-cli config set soniox
stt-cli config show              # confirm the key landed
stt-cli transcribe some-recording.m4a
```

If you skip the key, `stt-cli` tells you exactly what to run rather than failing
with an HTTP error.

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
| File under 25 MB, want it done in one request | `-p openai` |
| Long recording, or over 25 MB | `-p soniox` |
| You want per-token timing accuracy | `-p soniox` |
| Mixed or uncertain language | either, with `-l` omitted |

Set the one you reach for most as the default and stop typing `-p`:

```sh
stt-cli config default soniox
```

Both keys can live side by side; `--provider` overrides the default per run.

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

### Korean, or any specific language

```sh
stt-cli transcribe 20260815_143000_회의.m4a -l ko
```

The hint mainly helps short or noisy recordings. Leave it off for meetings that
switch between languages.

### Recordings over 25 MB on OpenAI

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
| Overrides | `OPENAI_API_KEY`, `SONIOX_API_KEY` |

Nothing else is written to disk — no cache, no history, no logs. Audio sent to
Soniox is deleted from Soniox once the transcript has been fetched; OpenAI's
retention is governed by your account settings.

## Troubleshooting

| Message | What it means |
|---|---|
| `no API key for openai.` | No key stored or exported. The message lists both ways to fix it. |
| `… is not a readable file` | Wrong path, or a directory was passed. |
| `cannot read a date and time from "yesterday"` | `--start` needs a real date, e.g. `"2026-08-15 14:30"`. |
| `… is 30.0 MB — OpenAI accepts at most 25 MB` | Use `-p soniox`, or split with ffmpeg. |
| `openai transcription failed (401)` | The key is wrong or revoked. |
| `soniox did not finish within 30 minutes` | The job is stuck; retry, or split the file. |
| `! no speech was recognised` | Silence, an unsupported codec, or the wrong `-l` hint. |
| `! no date or time in "…"` | Expected for un-dated names — pass `--start` if you need absolute times. |

Colours disappear when you pipe output; that is deliberate, and `NO_COLOR=1`
turns them off in a terminal too.

## Keeping it current

A Homebrew `--HEAD` install is upgraded by reinstalling it, not by `brew
upgrade`:

```sh
brew reinstall channprj/tap/stt-cli   # rebuilds from the latest main
brew list --versions stt-cli          # => stt-cli HEAD-394f304
brew uninstall stt-cli
```

`brew upgrade` reports "already installed" and `brew outdated` reports the
opposite — both are meaningless for a formula with no stable version. The commit
hash from `brew list --versions` is the reliable answer to "what am I running?".

From a source checkout, `git pull && cargo install --path .` does the same job.
