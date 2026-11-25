#!/usr/bin/env bash
#
# Cuts a release: bumps the workspace version from the conventional commits since the last tag, regenerates
# CHANGELOG.md, commits, tags and (after asking) pushes. The pushed tag starts the release workflow, which
# builds the Linux and Windows archives and attaches them to a GitHub release.
#
# Usage: scripts/release_version.sh

set -e

# Function to print errors only
err() { echo "$@" 1>&2; }

cd "$(dirname "$0")/.."

# enable push with follow tags
git config --global push.followTags true

# Check if git-cliff is installed
if ! command -v git-cliff &> /dev/null; then
    err "git-cliff is not installed. Please install it first."
    exit 1
fi

# Check if cargo-verset is installed
if ! command -v cargo-verset &> /dev/null; then
    err "cargo-verset is not installed. Please install it with:"
    err "    cargo install cargo-verset"
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    err "The working tree has uncommitted changes. Commit or stash them first."
    exit 1
fi

# git-cliff reads pull request data from GitHub; with a token it is not cut off by the rate limit.
if [ -z "${GITHUB_TOKEN:-}" ] && command -v gh &> /dev/null; then
    GITHUB_TOKEN="$(gh auth token 2> /dev/null || true)"
    export GITHUB_TOKEN
fi

# Get the bumped version from git-cliff; there is no current version before the first release.
version=$(git-cliff --bumped-version)
current_version=$(git describe --tags --abbrev=0 2> /dev/null || true)

# if the version is the same as the current version, exit
if [ "$version" == "$current_version" ]; then
    echo "Version $version is already the current version. No changes made."
    exit 0
fi

echo "Calculated version: $version"
echo "Updating version in Cargo.toml..."
cargo verset package "$version"
echo "Version updated successfully in Cargo.toml."

# Generate the changelog
echo "Generating changelog..."
git cliff --output CHANGELOG.md -t "$version"
echo "Changelog generated successfully."

echo "Building the project to update Cargo.lock..."
if ! cargo build --release --quiet; then
    err "Build failed. Please fix the issues and try again."
    exit 1
fi
echo "Project built successfully."

# Ask for confirmation before committing
echo "Please review the changes in Cargo.toml, Cargo.lock and CHANGELOG.md."
echo "Do you want to commit the changes? (y/n)"
read -r answer
if [ "$answer" != "y" ]; then
    echo "Changes not committed. Exiting."
    exit 0
fi

# Commit changes
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release($version)"
git tag -a "$version" -m "Release $version"

echo "Changes committed and tagged with version $version."
echo "Don't forget to push the changes to the remote repository."

# Ask for confirmation before pushing
echo "Do you want to push the changes? (y/n)"
read -r answer
if [ "$answer" != "y" ]; then
    echo "Changes not pushed. Exiting."
    exit 0
fi
git push
echo "Changes pushed successfully."
