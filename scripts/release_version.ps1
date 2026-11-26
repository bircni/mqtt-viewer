# Cuts a release: checks that the commit is fit to release, bumps the workspace version, regenerates
# CHANGELOG.md, commits, tags and (after asking) pushes. The pushed tag starts the Publish workflow, which
# builds the Linux and Windows archives and attaches them to a GitHub release.
#
# Usage: scripts/release_version.ps1 [version] [-SkipChecks]
#
# Without a version it follows from the conventional commits since the last tag. Pass one to choose it
# yourself, as for the step to 1.0.0, which no commit type asks for:
#
#     scripts/release_version.ps1 1.0.0
#
# Before anything is changed, the script starts the CI, Platforms and Publish workflows on the commit and
# waits for them; Publish, started by hand, builds every release file without releasing. -SkipChecks leaves
# that out.

param(
    [string]$Version,
    [switch]$SkipChecks
)

Set-Location (Join-Path $PSScriptRoot "..")

foreach ($tool in "git-cliff", "cargo-verset", "gh") {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Write-Host "$tool is not installed. Please install it first."
        exit 1
    }
}

# Only a clean main that matches the remote is released: the tag must name a commit CI has seen.
if ((git branch --show-current) -ne "main") {
    Write-Host "Releases are cut from main."
    exit 1
}
if (git status --porcelain) {
    Write-Host "The working tree has uncommitted changes. Commit or stash them first."
    exit 1
}
git fetch --quiet --tags origin main
$commit = git rev-parse HEAD
if ($commit -ne (git rev-parse origin/main)) {
    Write-Host "main and origin/main differ. Push or pull first."
    exit 1
}

# git-cliff reads pull request data from GitHub; with a token it is not cut off by the rate limit.
if (-not $env:GITHUB_TOKEN) {
    $env:GITHUB_TOKEN = gh auth token 2>$null
}

$version = if ($Version) { $Version } else { git cliff --bumped-version }
if ($version -notmatch '^\d+\.\d+\.\d+$') {
    Write-Host "'$version' is not a version like 1.2.3."
    exit 1
}
git rev-parse --quiet --verify "refs/tags/$version" | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "The tag $version exists already."
    exit 1
}
Write-Host "Version to release: $version"

# Starts a workflow on the commit and returns the identifier of its run.
function Start-Workflow([string]$workflow) {
    $started = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    gh workflow run $workflow --ref main | Out-Null
    foreach ($attempt in 1..30) {
        Start-Sleep -Seconds 2
        $run = gh run list --workflow $workflow --event workflow_dispatch --commit $commit `
            --created ">=$started" --limit 1 --json databaseId --jq '.[0].databaseId // empty'
        if ($run) {
            return $run
        }
    }
    Write-Host "The run of $workflow did not appear."
    exit 1
}

if (-not $SkipChecks) {
    Write-Host "Starting the CI, Platforms and Publish workflows on $commit..."
    $runs = @()
    foreach ($workflow in "ci.yml", "platforms.yml", "publish.yml") {
        $runs += Start-Workflow $workflow
    }
    foreach ($run in $runs) {
        gh run watch $run --exit-status | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Host "A workflow failed: $(gh run view $run --json url --jq .url)"
            Write-Host "Nothing was changed."
            exit 1
        }
    }
    Write-Host "All workflows passed."
}

Write-Host "Updating version in Cargo.toml..."
cargo verset package $version

Write-Host "Generating changelog..."
git cliff --output CHANGELOG.md -t $version

Write-Host "Building the project to update Cargo.lock..."
cargo build --release --quiet
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed. Please fix the errors and try again."
    exit 1
}

Write-Host "Please review the changes in Cargo.toml, Cargo.lock and CHANGELOG.md."
$confirmation = Read-Host "Do you want to commit the changes? (y/n)"
if ($confirmation -ne 'y') {
    Write-Host "Changes not committed. Exiting."
    exit 0
}

git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release($version)"
git tag -a $version -m "Release $version"
Write-Host "Changes committed and tagged with version $version."

$confirmation = Read-Host "Do you want to push the changes? (y/n)"
if ($confirmation -ne 'y') {
    Write-Host "Changes not pushed. Push them with: git push --atomic origin main $version"
    exit 0
}
# Both or neither: a tag without its commit on main would release something main does not contain.
git push --atomic origin main $version
Write-Host "Changes pushed successfully. The Publish workflow builds the release now."
