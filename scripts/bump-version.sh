#!/bin/bash
set -e

# Get current version from Cargo.toml
CURRENT=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')

IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"
PATCH=$((PATCH + 1))
NEW_VERSION="$MAJOR.$MINOR.$PATCH"

echo "Bumping version from $CURRENT to $NEW_VERSION"

sed -i "s/^version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" Cargo.toml
sed -i "s/^version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" src-shared/Cargo.toml
sed -i "s/^version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" src-tauri/Cargo.toml
sed -i "s/\"version\": \"$CURRENT\"/\"version\": \"$NEW_VERSION\"/" src-tauri/tauri.conf.json

git add Cargo.toml src-shared/Cargo.toml src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump version to $NEW_VERSION"
