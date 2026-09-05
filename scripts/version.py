#!/usr/bin/env python3
"""Synchronize and verify the Headatever version used by release consumers."""

import argparse
import datetime
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
VERSION_PATTERN = re.compile(r"(?:0|[1-9][0-9]*)\.([0-9]{6})\.(?:0|[1-9][0-9]*)")


def validate(version: str) -> str:
    match = VERSION_PATTERN.fullmatch(version)
    if not match:
        raise ValueError(f"invalid Headatever version: {version!r}; expected head.yymmdd.patch")
    datetime.datetime.strptime(match[1], "%y%m%d")
    return version


def current(root: Path = ROOT) -> str:
    return validate((root / "VERSION").read_text().strip())


def sync(root: Path = ROOT) -> str:
    version = current(root)
    patterns = {
        "Cargo.toml": r'(?ms)(^\[package\]\n(?:(?!^\[).)*?^version = ")[^"]+("$)',
        "Cargo.lock": r'(?m)(^\[\[package\]\]\nname = "stt-cli"\nversion = ")[^"]+("$)',
    }
    updates = {}
    for name, pattern in patterns.items():
        path = root / name
        old = path.read_text()
        new, count = re.subn(pattern, lambda match: match[1] + version + match[2], old)
        if count != 1:
            raise ValueError(f"expected one stt-cli version in {name}, found {count}")
        updates[path] = new
    for path, text in updates.items():
        path.write_text(text)
    check(root)
    return version


def check(root: Path = ROOT, *, binary: Path | None = None, tag: str | None = None) -> str:
    version = current(root)
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    packages = tomllib.loads((root / "Cargo.lock").read_text())["package"]
    locked = [package["version"] for package in packages if package["name"] == "stt-cli"]
    if manifest["package"]["version"] != version or locked != [version]:
        raise ValueError("Cargo.toml/Cargo.lock disagree with VERSION; run scripts/version.py sync")
    if tag is not None and tag != f"v{version}":
        raise ValueError(f"tag {tag!r} does not match VERSION v{version}")
    if binary is not None:
        output = subprocess.check_output([str(binary.resolve()), "--version"], text=True).strip()
        if output != f"stt-cli {version}":
            raise ValueError(f"binary reports {output!r}; expected 'stt-cli {version}'")
    return version


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("sync")
    verify = sub.add_parser("check")
    verify.add_argument("--binary", type=Path)
    verify.add_argument("--tag")
    validation = sub.add_parser("validate")
    validation.add_argument("version")
    args = parser.parse_args()
    if args.command == "sync":
        print(sync())
    elif args.command == "check":
        print(check(binary=args.binary, tag=args.tag))
    else:
        print(validate(args.version))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"version: {error}") from error
