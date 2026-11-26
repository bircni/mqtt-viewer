#!/usr/bin/env bash
#
# Cuts a release: checks that the commit is fit to release, bumps the workspace version, regenerates
# CHANGELOG.md, commits, tags and (after asking) pushes. The pushed tag starts the Publish workflow, which
# builds the Linux and Windows archives and attaches them to a GitHub release.
#
# Usage: scripts/release_version.sh [version] [--skip-checks]
#
# Without a version it follows from the conventional commits since the last tag. Pass one to choose it
# yourself, as for the step to 1.0.0, which no commit type asks for:
#
#     scripts/release_version.sh 1.0.0
#
# Before anything is changed, the script starts the CI, Platforms and Publish workflows on the commit and
# waits for them; Publish, started by hand, builds every release file without releasing. --skip-checks leaves
# that out.

set -euo pipefail

err() { echo "$@" 1>&2; }

cd "$(dirname "$0")/.."

version=""
skip_checks=false
for argument in "$@"; do
    case "$argument" in
        --skip-checks) skip_checks=true ;;
        -*) err "Unknown option $argument."; exit 1 ;;
        *) version="$argument" ;;
    esac
done

for tool in git-cliff cargo-verset gh; do
    if ! command -v "$tool" &> /dev/null; then
        err "$tool is not installed. Please install it first."
        exit 1
    fi
done

# Only a clean main that matches the remote is released: the tag must name a commit CI has seen.
if [ "$(git branch --show-current)" != "main" ]; then
    err "Releases are cut from main."
    exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
    err "The working tree has uncommitted changes. Commit or stash them first."
    exit 1
fi
git fetch --quiet --tags origin main
commit="$(git rev-parse HEAD)"
if [ "$commit" != "$(git rev-parse origin/main)" ]; then
    err "main and origin/main differ. Push or pull first."
    exit 1
fi

# git-cliff reads pull request data from GitHub; with a token it is not cut off by the rate limit.
if [ -z "${GITHUB_TOKEN:-}" ]; then
    GITHUB_TOKEN="$(gh auth token 2> /dev/null || true)"
    export GITHUB_TOKEN
fi

if [ -z "$version" ]; then
    version="$(git-cliff --bumped-version)"
fi
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    err "'$version' is not a version like 1.2.3."
    exit 1
fi
if git rev-parse --quiet --verify "refs/tags/$version" > /dev/null; then
    err "The tag $version exists already."
    exit 1
fi
echo "Version to release: $version"

# Starts a workflow on the commit and prints the identifier of its run.
start_workflow() {
    local workflow="$1" started run
    started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    gh workflow run "$workflow" --ref main > /dev/null
    for _ in $(seq 1 30); do
        sleep 2
        run="$(gh run list --workflow "$workflow" --event workflow_dispatch --commit "$commit" \
            --created ">=$started" --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
        if [ -n "$run" ]; then
            echo "$run"
            return 0
        fi
    done
    err "The run of $workflow did not appear."
    return 1
}

if [ "$skip_checks" = false ]; then
    echo "Starting the CI, Platforms and Publish workflows on $commit..."
    runs=()
    for workflow in ci.yml platforms.yml publish.yml; do
        runs+=("$(start_workflow "$workflow")")
    done
    for run in "${runs[@]}"; do
        if ! gh run watch "$run" --exit-status > /dev/null; then
            err "A workflow failed: $(gh run view "$run" --json url --jq .url)"
            err "Nothing was changed."
            exit 1
        fi
    done
    echo "All workflows passed."
fi

echo "Updating version in Cargo.toml..."
cargo verset package "$version"

echo "Generating changelog..."
git cliff --output CHANGELOG.md -t "$version"

echo "Building the project to update Cargo.lock..."
if ! cargo build --release --quiet; then
    err "Build failed. Please fix the issues and try again."
    exit 1
fi

echo "Please review the changes in Cargo.toml, Cargo.lock and CHANGELOG.md."
echo "Do you want to commit the changes? (y/n)"
read -r answer
if [ "$answer" != "y" ]; then
    echo "Changes not committed. Exiting."
    exit 0
fi

git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release($version)"
git tag -a "$version" -m "Release $version"
echo "Changes committed and tagged with version $version."

echo "Do you want to push the changes? (y/n)"
read -r answer
if [ "$answer" != "y" ]; then
    echo "Changes not pushed. Push them with: git push --atomic origin main $version"
    exit 0
fi
# Both or neither: a tag without its commit on main would release something main does not contain.
git push --atomic origin main "$version"
echo "Changes pushed successfully. The Publish workflow builds the release now."
