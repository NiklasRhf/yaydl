#!/usr/bin/env bash
# Bumps the app version in every manifest, commits, and tags. Pushing the tag
# triggers the release workflow. Nothing is pushed by this script.
set -euo pipefail

version="${1:-}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: scripts/release.sh <major.minor.patch>" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

if [ -n "$(git status --porcelain)" ]; then
  echo "working tree is not clean" >&2
  exit 1
fi
if [ "$(git branch --show-current)" != "main" ]; then
  echo "release from main, not $(git branch --show-current)" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/yaydl-v$version" >/dev/null; then
  echo "tag yaydl-v$version already exists" >&2
  exit 1
fi

current=$(python3 -c "import json; print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
sed -i "0,/^version = \"$current\"/s//version = \"$version\"/" Cargo.toml src-shared/Cargo.toml src-tauri/Cargo.toml
sed -i "s/\"version\": \"$current\"/\"version\": \"$version\"/" src-tauri/tauri.conf.json
cargo update --workspace --quiet

for f in Cargo.toml src-shared/Cargo.toml src-tauri/Cargo.toml src-tauri/tauri.conf.json; do
  grep -q "\"$version\"" "$f" || { echo "$f was not updated" >&2; exit 1; }
done

git add Cargo.toml Cargo.lock src-shared/Cargo.toml src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -q -m "chore: release v$version"
git tag -a "yaydl-v$version" -m "YaYDL v$version"

echo "committed and tagged yaydl-v$version. To release:"
echo "  git push origin main yaydl-v$version"
