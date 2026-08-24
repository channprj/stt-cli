# Recursive Batch Transcription Design

**Date:** 2026-08-24
**Status:** Approved

## Summary

`stt-cli` will gain a dedicated `batch` command that recursively discovers
audio files beneath one input directory and writes one transcript per file into
one flat output directory. Output names retain the input file stem. Existing
files and names duplicated within the batch are never overwritten; the command
adds a numeric suffix such as `-2` or `-3` instead.

The same change adds Markdown as a first-class transcript format for both
single-file and batch transcription.

## Goals

- Transcribe every supported audio file anywhere below a selected directory.
- Put every result directly in the selected output directory, regardless of
  the input directory structure.
- Preserve input file stems as closely as possible while preventing all
  overwrites.
- Keep processing after a file-specific failure and report partial success
  clearly.
- Reuse one transcription and rendering path for single-file and batch modes.
- Support Markdown output with useful transcript and source metadata.
- Preserve the current single-file `transcribe` command and its defaults.

## Non-goals

- Concurrent API requests, rate-limit scheduling, resumable queues, or durable
  job state.
- Watching directories for newly added files.
- Content-based media detection or transcoding unsupported codecs.
- Recreating the input directory tree beneath the output directory.
- Overwrite, replace, or force flags.

## Command-line interface

The new command is:

```console
stt-cli batch <INPUT_DIR> --output-dir <OUTPUT_DIR> [OPTIONS]
```

`--output-dir` also has the short form `-o`. Batch mode accepts the existing
provider, model, language, format, dry-run, and VAD controls:

```console
stt-cli batch ~/Recordings -o ~/Transcripts
stt-cli batch ~/Recordings -o ~/Transcripts -p soniox -l ko --vad
stt-cli batch ~/Recordings -o ~/Transcripts -f markdown
stt-cli batch ~/Recordings -o ~/Transcripts --dry-run
```

Batch mode does not accept `--start`. A single start time would be misleading
for multiple recordings, so each file independently derives its anchor from
its own name. A recording without a recognizable date and time retains the
existing relative-timestamp fallback.

The existing command remains unchanged:

```console
stt-cli transcribe <FILE> [OPTIONS]
```

## Audio discovery

Discovery recursively walks the input root and selects regular files with a
known audio or audio-capable media extension, compared case-insensitively. The
initial set is:

```text
aac, aif, aiff, alac, amr, ape, au, caf, flac, m4a, m4b, mp3, mp4,
mpeg, mpga, oga, ogg, opus, wav, wave, webm, wma
```

The extension list determines discovery, not whether every provider accepts
every codec. A provider rejection is handled as a file-specific failure and
does not stop the rest of the batch.

Nested directory symlinks are not followed. This prevents directory cycles and
unexpected traversal outside the requested tree. Unsupported files and
symlinked files are ignored.

Discovered files are sorted by their path relative to the input root before
processing. This makes output allocation, progress, and test results stable.

If an existing output directory is a proper descendant of the input root, its
entire subtree is excluded from discovery. If input and output are the same
directory, the root is still scanned; generated transcript extensions are not
audio extensions, so results cannot recursively become new inputs.

Finding no supported audio files is an error. In this case the command does not
create the output directory.

## Flat output and collision handling

Each output starts with the full input file stem and uses the extension for the
selected transcript format:

| Format | Output extension |
|---|---|
| `text` | `.txt` |
| `txt` | `.txt` |
| `markdown` / `md` | `.md` |
| `json` | `.json` |
| `srt` | `.srt` |
| `vtt` | `.vtt` |
| `csv` | `.csv` |

Examples:

```text
input/team-a/meeting.m4a     -> output/meeting.txt
input/team-b/meeting.wav     -> output/meeting-2.txt
input/archive/meeting.mp3    -> output/meeting-3.txt
```

An occupied name can come from an existing file, an existing directory, or an
earlier planned item in the current batch. Name comparison for allocation is
case-insensitive so a batch remains safe when moved between common macOS,
Windows, and Linux filesystems. The unsuffixed name is preferred; otherwise the
lowest available suffix beginning at `-2` is selected.

The command must also defend against another process creating the planned name
after discovery. Final output uses create-new semantics. If the name became
occupied, the command allocates the next available suffix and retries rather
than overwriting. A write failure removes the incomplete file when possible and
is reported as a failure for that input.

Every successful item prints its actual input-to-output mapping. Numeric naming
is deterministic for a stable directory snapshot because input traversal is
sorted.

## Shared transcription pipeline

`main.rs` remains the CLI composition root. Batch-specific discovery, output
planning, and summary behavior live behind a focused batch module. The current
single-file orchestration is separated into reusable operations:

1. Resolve provider, model, credentials, language, and VAD settings.
2. Derive the file-specific start-time anchor.
3. Optionally build VAD-compressed media and retain its offset map.
4. Call the selected provider and remap returned segments.
5. Render the requested format with source metadata.
6. Return rendered content to the command-specific output writer.

`transcribe` writes the returned content to its explicit output file or stdout.
`batch` selects a collision-free output path and writes the content there. The
provider adapters and their normalized `Segment` contract do not need to know
which command initiated the request.

Batch execution is sequential. This avoids surprising API bursts, preserves
deterministic progress, and fits the project's synchronous runtime. Provider
configuration and credentials are resolved once before the first real API
request. A missing configuration or API key therefore fails fast instead of
repeating the same error for every file.

VAD temporary media is cleaned after every item on both success and error
paths. One failed item cannot leak temporary state into the next item.

## Markdown format

`Format` gains the canonical value `markdown` with `md` as a command-line
alias. A Markdown transcript uses this structure:

```markdown
# meeting.m4a

- **Source:** `team-a/meeting.m4a`
- **Provider:** `openai`
- **Model:** `whisper-1`
- **Started:** `2026-08-24 14:30:00`

## Transcript

- **[2026-08-24 14:30:05]** Good morning.
- **[2026-08-24 14:30:19]** **Speaker 2:** Let us begin.
```

The `Started` row is omitted when no anchor is known. Segment timestamps use
absolute wall-clock values when anchored and relative `HH:MM:SS` offsets
otherwise, matching the existing `text` format. Speaker labels are emitted only
when present. The source value is the relative input path in batch mode and the
input file name in single-file mode, which helps identify numerically suffixed
flat outputs.

The Markdown renderer always emits the title, metadata, and `Transcript`
heading, even when the provider recognizes no speech.

## Dry-run behavior

`batch --dry-run` performs discovery, stable sorting, collision planning, and
the existing per-file request preview. It does not:

- load or require an API key;
- call a transcription provider;
- create the output directory;
- reserve or write output files; or
- create VAD-compressed media.

The preview shows every planned input-to-output mapping. Existing dry-run
duration and VAD cost estimation may invoke `ffprobe` or silence detection, as
the single-file command already does, but these operations do not create the
batch output directory.

## Progress, failures, and exit status

For a real batch, each file has an independent success or failure result.
File-specific discovery, anchor, VAD, provider, render, or write errors are
printed with the input path, and processing continues with the next file.
Successful transcripts remain in place.

The final summary reports total, succeeded, and failed counts. It also lists
failed input paths with their error messages. The command returns a failing
exit status if any item failed, including a write failure after a successful
provider response. It succeeds only when every discovered file produced a
transcript.

Errors that make useful per-file work impossible fail before iteration:

- the input path is missing or is not a readable directory;
- the output path exists but is not a directory;
- no supported audio files were found; or
- real execution cannot resolve a provider or its API key.

## Testing strategy

Unit and command-level tests cover:

- recursive discovery through multiple nesting levels;
- case-insensitive audio extensions;
- ignored unsupported files and nested directory symlinks;
- exclusion of a nested output subtree;
- stable relative-path ordering;
- flat output names for nested inputs;
- collisions between equal stems with different audio extensions;
- collisions with existing files and directories;
- skipping occupied numeric suffixes;
- case-insensitive planned-name collisions;
- create-new retry when a planned output is occupied concurrently;
- output extension mapping for every format;
- Markdown title, metadata, absolute and relative timestamps, speakers, and
  empty transcripts;
- per-file failure continuation and failing final status;
- fail-fast configuration errors;
- dry-run mapping without output-directory or output-file creation; and
- argument parsing for the new command and Markdown alias.

A built-binary smoke test uses nested, empty files with recognized audio
extensions and `batch --dry-run`. It verifies recursive discovery, flat output
mapping, collision suffixes, and the absence of filesystem writes without
requiring credentials or live API calls.

Every implementation checkpoint must also pass the repository gates:

```console
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

## Documentation changes

`README.md` and `README.ko.md` will add batch transcription and Markdown to the
feature overview and quick start. `USAGE.md` will replace the shell-loop-only
folder recipe with the native recursive command, document naming collisions,
failure semantics, dry-run behavior, supported extensions, and Markdown output.
`ARCHITECTURE.md` will describe the batch module and the shared single-file
pipeline.

## Acceptance criteria

The feature is complete when all of the following are proven from the current
worktree and built CLI:

1. One command recursively discovers supported audio files at arbitrary depths.
2. Every successful transcript is written directly beneath the requested
   output directory.
3. Existing output and same-stem batch collisions are preserved with numeric
   suffixes and no overwrite path exists.
4. One item failure does not prevent later items from being attempted, while
   the final exit status still reports partial failure.
5. Markdown works in both `transcribe` and `batch` with `.md` batch outputs.
6. Dry-run proves discovery and naming without API calls or output mutations.
7. Existing single-file behavior and formats remain compatible.
8. Unit tests, Clippy, formatting, diff checks, and built-CLI smoke verification
   all pass.
