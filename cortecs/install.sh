#!/usr/bin/env sh
# Build "Codex for cortecs.ai" from this checkout and install it as `cortecs-codex`.
#
# Usage: cortecs/install.sh [--prefix DIR] [--debug]
#        cortecs/install.sh --protect-secrets
#   --prefix DIR        install into DIR/bin (default: $HOME/.local)
#   --debug             faster, unoptimized build (useful while hacking on the fork)
#   --protect-secrets   only set up /etc/codex/requirements.toml (needs sudo) so that
#                       sandboxed commands cannot read credentials (~/.ssh, ~/.aws, ...)
#
# Environment: PREFIX (same as --prefix), CARGO (cargo executable).
set -eu

prefix="${PREFIX:-$HOME/.local}"
profile="release"
protect_secrets=0
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) prefix="$2"; shift 2 ;;
    --prefix=*) prefix="${1#--prefix=}"; shift ;;
    --debug) profile="dev"; shift ;;
    --protect-secrets) protect_secrets=1; shift ;;
    -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# Managed requirements apply to every Codex on this machine (all users, upstream Codex
# too). The deny list survives switching modes in /permissions; only full access, which
# has no sandbox, ignores it. "~" expands to each user's home directory.
requirements_file="${CODEX_REQUIREMENTS_FILE:-/etc/codex/requirements.toml}"
begin_marker="# >>> cortecs-codex protect-secrets >>>"
end_marker="# <<< cortecs-codex protect-secrets <<<"

secrets_block() {
  cat <<BLOCK
$begin_marker
# Sandboxed commands cannot read these, even after you approve running them with
# more permissions. Managed by cortecs/install.sh --protect-secrets; the lines
# between the markers are replaced when it runs again.
[permissions.filesystem]
deny_read = [
  "~/.ssh", "~/.gnupg", "~/.password-store", "~/.local/share/keyrings",
  "~/.aws", "~/.azure", "~/.config/gcloud", "~/.kube",
  "~/.netrc", "~/.git-credentials", "~/.config/gh", "~/.docker/config.json",
  "~/.pypirc", "~/.cargo/credentials.toml",
  "~/.mozilla", "~/.config/google-chrome", "~/.config/chromium",
  "~/.cortecs-codex/auth.json", "~/.codex/auth.json",
]
$end_marker
BLOCK
}

protect_secrets() {
  tmp=$(mktemp)
  trap 'rm -f "$tmp"' EXIT
  if [ ! -e "$requirements_file" ]; then
    secrets_block > "$tmp"
  elif grep -qF "$begin_marker" "$requirements_file"; then
    # Replace our previous block and keep everything else.
    awk -v begin="$begin_marker" -v end="$end_marker" '
      $0 == begin { skip = 1; next }
      $0 == end { skip = 0; next }
      !skip { print }
    ' "$requirements_file" > "$tmp"
    secrets_block >> "$tmp"
  elif grep -qE '^[[:space:]]*(\[+[[:space:]]*permissions|permissions[[:space:]]*[.=])' "$requirements_file"; then
    echo "error: $requirements_file already configures permissions; merge this by hand:" >&2
    echo >&2
    secrets_block >&2
    exit 1
  else
    cat "$requirements_file" > "$tmp"
    printf '\n' >> "$tmp"
    secrets_block >> "$tmp"
  fi

  # Every user's Codex reads this file, so the directory must be world-readable no
  # matter the umask (sudo often keeps a restrictive one). `install -d -m` also fixes
  # the mode of an existing directory.
  requirements_dir=$(dirname "$requirements_file")
  if ! { install -d -m 755 "$requirements_dir" && install -m 644 "$tmp" "$requirements_file"; } 2>/dev/null; then
    if ! command -v sudo >/dev/null 2>&1; then
      echo "error: writing $requirements_file needs root, and sudo is not available" >&2
      exit 1
    fi
    echo "==> Writing $requirements_file (asks for your sudo password)"
    sudo install -d -m 755 "$requirements_dir"
    sudo install -m 644 "$tmp" "$requirements_file"
  fi
  echo "==> Sandboxed commands can no longer read credentials; see $requirements_file"
}

if [ "$protect_secrets" -eq 1 ]; then
  protect_secrets
  exit 0
fi

cargo="${CARGO:-cargo}"
if ! command -v "$cargo" >/dev/null 2>&1; then
  echo "error: Rust is required to build cortecs-codex. Install it from https://rustup.rs" >&2
  exit 1
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root/codex-rs"

echo "==> Building cortecs-codex ($profile profile); the first build takes a while..."
"$cargo" build --profile "$profile" -p codex-cli --bin codex

target_dir="${CARGO_TARGET_DIR:-$repo_root/codex-rs/target}"
case "$profile" in
  dev) built="$target_dir/debug/codex" ;;
  *) built="$target_dir/$profile/codex" ;;
esac

bin_dir="$prefix/bin"
mkdir -p "$bin_dir"
dest="$bin_dir/cortecs-codex"
# Copy to a temporary name first so a running instance is never truncated.
cp "$built" "$dest.tmp.$$"
chmod 755 "$dest.tmp.$$"
if [ "$profile" != "dev" ] && command -v strip >/dev/null 2>&1; then
  strip "$dest.tmp.$$" 2>/dev/null || true
fi
mv -f "$dest.tmp.$$" "$dest"
echo "==> Installed $dest"

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) echo "note: $bin_dir is not on your PATH; add it, e.g.: export PATH=\"$bin_dir:\$PATH\"" ;;
esac

if ! grep -qsF "$begin_marker" "$requirements_file"; then
  cat <<EOF

Recommended: keep sandboxed commands away from your credentials (~/.ssh, ~/.aws, ...):
  $0 --protect-secrets     # writes $requirements_file (needs sudo)
EOF
fi

cat <<EOF

Next steps:
  cortecs-codex            # start; asks for your cortecs.ai API key on first run
  cortecs-codex login      # or store the key up front
  export CORTECS_API_KEY=… # or provide it via the environment
EOF
