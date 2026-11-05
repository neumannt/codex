#!/usr/bin/env sh
# Build "Codex for cortecs.ai" from this checkout and install it as `cortecs-codex`.
#
# Usage: cortecs/install.sh [--prefix DIR] [--debug]
#   --prefix DIR   install into DIR/bin (default: $HOME/.local)
#   --debug        faster, unoptimized build (useful while hacking on the fork)
#
# Environment: PREFIX (same as --prefix), CARGO (cargo executable).
set -eu

prefix="${PREFIX:-$HOME/.local}"
profile="release"
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) prefix="$2"; shift 2 ;;
    --prefix=*) prefix="${1#--prefix=}"; shift ;;
    --debug) profile="dev"; shift ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

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

cat <<EOF

Next steps:
  cortecs-codex            # start; asks for your cortecs.ai API key on first run
  cortecs-codex login      # or store the key up front
  export CORTECS_API_KEY=… # or provide it via the environment
EOF
