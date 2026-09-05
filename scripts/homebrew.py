#!/usr/bin/env python3
"""Render a stable source formula pinned to an authenticated Git tag and commit."""

import argparse
from pathlib import Path
import re

import version


def render(tag: str, revision: str) -> str:
    if not tag.startswith("v"):
        raise ValueError("release tag must start with v")
    version.validate(tag[1:])
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be a full lowercase Git commit SHA")
    return f'''class SttCli < Formula
  desc "Transcribe audio and stamp each line with the time it was spoken"
  homepage "https://github.com/channprj/stt-cli"
  url "https://github.com/channprj/stt-cli.git",
      tag:      "{tag}",
      revision: "{revision}"
  license :cannot_represent
  head "https://github.com/channprj/stt-cli.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      Source access to channprj/stt-cli is required for installs and upgrades.
      Authenticate Git with gh auth login followed by gh auth setup-git.
      VAD is optional; install ffmpeg separately to use --vad.
    EOS
  end

  test do
    ENV["XDG_CONFIG_HOME"] = testpath
    output = shell_output("#{{bin}}/stt-cli --version")
    assert_match(/^stt-cli \\d+\\.\\d{{6}}\\.\\d+\\n$/, output)
    assert_equal "stt-cli #{{version}}\\n", output unless build.head?
    assert_match "stt-cli/api.json", shell_output("#{{bin}}/stt-cli config path")
    audio = testpath/"20260101_090000.m4a"
    audio.write ""
    preview = shell_output("#{{bin}}/stt-cli transcribe #{{audio}} -p openai -f txt --dry-run 2>&1")
    assert_match "format:    Txt", preview
    assert_match "2026-01-01 09:00:00", preview
  end
end
'''


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        formula = render(args.tag, args.revision)
        if args.output:
            args.output.write_text(formula)
        else:
            print(formula, end="")
    except (ValueError, OSError) as error:
        parser.error(str(error))
