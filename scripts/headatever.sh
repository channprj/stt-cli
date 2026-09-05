#!/usr/bin/env bash
#
# headatever — bump a Headatever version (head.yymmdd.patch).
#
# Adapted from the installed Headatever skill for stt-cli's Cargo metadata.
# Writes VERSION, synchronizes Cargo metadata, commits, and tags v<version>.
# Dates use local time. See: https://github.com/channprj/headatever
#
set -euo pipefail

# Structural validity: head (no leading zeros, 0 allowed), yymmdd (mm 01-12,
# dd 01-31), patch (no leading zeros, 0 allowed).
readonly VERSION_RE='^(0|[1-9][0-9]*)\.[0-9]{2}(0[1-9]|1[0-2])(0[1-9]|[12][0-9]|3[01])\.(0|[1-9][0-9]*)$'

dry_run=0
no_git=0
push=0
release=0

die() { printf 'headatever: %s\n' "$1" >&2; exit 1; }

# Ensure the GitHub CLI is available and authenticated before a release.
require_gh() {
  command -v gh >/dev/null 2>&1 \
    || die "gh (GitHub CLI) not found; install from https://cli.github.com"
  gh auth status >/dev/null 2>&1 \
    || die "gh is not authenticated; run 'gh auth login'"
}

usage() {
  cat <<'EOF'
Usage: headatever <command> [options]

Commands:
  show              Print the current version (read-only)
  major             Bump head: head+1, date=today, patch=0
  patch             Release again: same day -> patch+1, new day -> date=today, patch=0
  date              Start a new day: date=today, patch=0 (errors if already today)
  set <version>     Set an explicit head.yymmdd.patch version
  init [head]       Create VERSION (default head 0) if it does not exist
  push              Push the current release commit and tag (git push --follow-tags)
  release           Push the current commit/tag, then create a GitHub release (needs gh)

Options:
  --dry-run         Print what would change; write nothing, run no git
  --no-git          Write VERSION only; skip commit and tag
  --push            After commit + tag, run: git push --follow-tags
  --release         After commit + tag, push and create a GitHub release (implies --push)
  -h, --help        Show this help

By default a bump writes VERSION, commits it as "chore(release): v<version>",
and creates an annotated tag v<version>. The date uses local time.
The 'release' command and --release flag require the GitHub CLI (gh).
EOF
}

# Resolve the VERSION file at the git root, falling back to ./VERSION.
version_file() {
  local root
  if root=$(git rev-parse --show-toplevel 2>/dev/null); then
    printf '%s/VERSION' "$root"
  else
    printf './VERSION'
  fi
}

today() { date "+%y%m%d"; }

# Force base-10 so leading-zero fields are never read as octal.
num() { printf '%d' "$((10#$1))"; }

validate() {
  [[ $1 =~ $VERSION_RE ]] || die "invalid version: '$1' (expected head.yymmdd.patch)"
  python3 "$(dirname "$file")/scripts/version.py" validate "$1" >/dev/null
}

read_current() {
  local f=$1 v
  [[ -f $f ]] || die "no VERSION file at $f (run 'headatever init')"
  v=$(tr -d ' \t\r\n' < "$f")
  [[ -n $v ]] || die "VERSION file is empty: $f"
  # Accept the old VERSION spelling on input; every write is canonical.
  v=${v#v}
  validate "$v"
  printf '%s' "$v"
}

# --- parse arguments -------------------------------------------------------
args=()
while (($#)); do
  case $1 in
    --dry-run) dry_run=1 ;;
    --no-git)  no_git=1 ;;
    --push)    push=1 ;;
    --release) release=1 ;;
    -h|--help) usage; exit 0 ;;
    --) shift; while (($#)); do args+=("$1"); shift; done; break ;;
    -*) die "unknown option: $1 (try --help)" ;;
    *)  args+=("$1") ;;
  esac
  shift
done

[[ ${#args[@]} -ge 1 ]] || { usage; exit 1; }
cmd=${args[0]}
file=$(version_file)

# --- compute the new version ----------------------------------------------
old=""
case $cmd in
  show)
    read_current "$file"; echo; exit 0
    ;;
  push)
    (( no_git )) && die "'push' cannot be combined with --no-git"
    old=$(read_current "$file")
    tag="v$old"
    if (( dry_run )); then
      echo "[dry-run] push $tag"
      echo "[dry-run] git push --follow-tags"
      exit 0
    fi
    git rev-parse --show-toplevel >/dev/null 2>&1 \
      || die "not a git repository"
    git rev-parse -q --verify "refs/tags/$tag" >/dev/null \
      || die "tag $tag does not exist locally; run a bump first"
    git push --follow-tags
    echo "pushed $tag"
    exit 0
    ;;
  release)
    (( no_git )) && die "'release' cannot be combined with --no-git"
    old=$(read_current "$file")
    tag="v$old"
    if (( dry_run )); then
      echo "[dry-run] release $tag"
      echo "[dry-run] git push --follow-tags"
      echo "[dry-run] gh release create $tag --generate-notes --title $tag"
      exit 0
    fi
    git rev-parse --show-toplevel >/dev/null 2>&1 \
      || die "not a git repository"
    git rev-parse -q --verify "refs/tags/$tag" >/dev/null \
      || die "tag $tag does not exist locally; run a bump first"
    require_gh
    gh release view "$tag" >/dev/null 2>&1 \
      && die "GitHub release $tag already exists"
    git push --follow-tags
    gh release create "$tag" --generate-notes --title "$tag"
    echo "released $tag"
    exit 0
    ;;
  init)
    [[ -f $file ]] && die "VERSION already exists at $file"
    init_head=${args[1]:-0}
    [[ $init_head =~ ^(0|[1-9][0-9]*)$ ]] \
      || die "init head must be a non-negative integer with no leading zeros"
    new="$init_head.$(today).0"
    ;;
  major|patch|date|set)
    old=$(read_current "$file")
    IFS=. read -r ch cd cp <<<"$old"
    t=$(today)
    case $cmd in
      major)
        new="$(( $(num "$ch") + 1 )).$t.0"
        ;;
      date)
        if   (( $(num "$t") <  $(num "$cd") )); then
          die "system date $t is before VERSION date $cd; refusing to go backwards"
        elif (( $(num "$t") == $(num "$cd") )); then
          die "already on today ($cd); use 'patch' for another release today"
        fi
        new="$ch.$t.0"
        ;;
      patch)
        if   (( $(num "$t") <  $(num "$cd") )); then
          die "system date $t is before VERSION date $cd; refusing to go backwards"
        elif (( $(num "$t") == $(num "$cd") )); then
          new="$ch.$cd.$(( $(num "$cp") + 1 ))"
        else
          new="$ch.$t.0"
        fi
        ;;
      set)
        new=${args[1]:-}
        [[ -n $new ]] || die "set requires a version argument"
        ;;
    esac
    ;;
  *)
    die "unknown command: $cmd (try --help)"
    ;;
esac

validate "$new"

# No-op guard (e.g. `set` to the current value).
if [[ -n $old && $new == "$old" && $(tr -d ' \t\r\n' < "$file") == "$new" ]]; then
  echo "VERSION already $new (no change)"
  exit 0
fi

tag="v$new"

# --- dry run ---------------------------------------------------------------
if (( dry_run )); then
  echo "[dry-run] ${old:+$old -> }$new"
  echo "[dry-run] write $file"
  if (( ! no_git )); then
    echo "[dry-run] synchronize Cargo.toml and Cargo.lock"
    echo "[dry-run] git add -- VERSION Cargo.toml Cargo.lock"
    echo "[dry-run] git commit -m \"chore(release): $tag\" -- VERSION Cargo.toml Cargo.lock"
    echo "[dry-run] git tag -a $tag -m $tag"
    (( push || release )) && echo "[dry-run] git push --follow-tags"
    (( release )) && echo "[dry-run] gh release create $tag --generate-notes --title $tag"
  fi
  exit 0
fi

# --- apply -----------------------------------------------------------------
# Pre-flight git checks BEFORE mutating VERSION, so a failed bump leaves no
# half-applied change on disk.
if (( ! no_git )); then
  git rev-parse --show-toplevel >/dev/null 2>&1 \
    || die "not a git repository; re-run with --no-git"
  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    die "tag $tag already exists"
  fi
  (( release )) && require_gh
fi

printf '%s\n' "$new" > "$file"
python3 "$(dirname "$file")/scripts/version.py" sync >/dev/null

if (( no_git )); then
  echo "${old:+$old -> }$new (VERSION written; git skipped)"
  exit 0
fi

git add -- "$file" "$(dirname "$file")/Cargo.toml" "$(dirname "$file")/Cargo.lock"
git commit -q -m "chore(release): $tag" -- "$file" "$(dirname "$file")/Cargo.toml" "$(dirname "$file")/Cargo.lock"
git tag -a "$tag" -m "$tag"

suffix=""
if (( push || release )); then
  git push --follow-tags
  suffix=", pushed"
fi
if (( release )); then
  gh release create "$tag" --generate-notes --title "$tag"
  suffix="$suffix, released"
fi

echo "${old:+$old -> }$new  (committed, tagged $tag$suffix)"
