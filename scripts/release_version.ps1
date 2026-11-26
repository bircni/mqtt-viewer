# Cuts a release: bumps the workspace version from the conventional commits since the last tag, regenerates
# CHANGELOG.md, commits and tags. Pushing the tag starts the release workflow.
#
# Usage: scripts/release_version.ps1 [version]
#
# Without an argument the version follows from the commits. Pass one to choose it yourself, as for the step
# to 1.0.0, which no commit type asks for:
#
#     scripts/release_version.ps1 1.0.0

param([string]$Version)

Set-Location (Join-Path $PSScriptRoot "..")

# Enable push with follow tags
git config --global push.followTags true

# Check if git cliff is installed
if (-not (Get-Command git-cliff -ErrorAction SilentlyContinue)) {
    Write-Host "git cliff is not installed. Installing it now..."
    # Install git cliff using Winget
    winget install -e --id orhun.git-cliff -s winget
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Failed to install git cliff. Please install it manually."
        exit 1
    }
    Write-Host "git cliff installed successfully."
}
# Check if cargo-verset is installed
if (-not (Get-Command cargo-verset -ErrorAction SilentlyContinue)) {
    Write-Host "cargo-verset is not installed. Please install it with cargo install cargo-verset"
    exit 1
}

if (git status --porcelain) {
    Write-Host "The working tree has uncommitted changes. Commit or stash them first."
    exit 1
}

# git-cliff reads pull request data from GitHub; with a token it is not cut off by the rate limit.
if (-not $env:GITHUB_TOKEN -and (Get-Command gh -ErrorAction SilentlyContinue)) {
    $env:GITHUB_TOKEN = gh auth token 2>$null
}

$version = if ($Version) { $Version } else { git cliff --bumped-version }
if ($version -notmatch '^\d+\.\d+\.\d+$') {
    Write-Host "'$version' is not a version like 1.2.3."
    exit 1
}
# There is no current version before the first release.
$current_version = git describe --tags --abbrev=0 2>$null
# Check if the version is already the same as the current version
if ($version -eq $current_version) {
    Write-Host "Version is already set to $version. No changes made."
    exit 0
}

Write-Host "Version to release: $version"
Write-Host "Updating version in Cargo.toml..."
# Update the version in Cargo.toml
cargo verset package $version
Write-Host "Version updated successfully in Cargo.toml."
# Generate the changelog
Write-Host "Generating changelog..."
git cliff --output CHANGELOG.md -t $version
Write-Host "Changelog generated successfully."
# Update Cargo.lock
Write-Host "Building..."
cargo build --release --quiet
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed. Please fix the errors and try again."
    exit 1
}
Write-Host "Build succeeded."
# Ask for confirmation before committing
Write-Host "Please review the changes in Cargo.toml, Cargo.lock and CHANGELOG.md."
$confirmation = Read-Host "Do you want to commit the changes? (y/n)"
if ($confirmation -ne 'y') {
    Write-Host "Changes not committed. Exiting."
    exit 0
}
# Commit the changes
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release($version)"
git tag -a $version -m "Release $version"

Write-Host "Changes committed and tagged with version $version."
Write-Host "Please push the changes to the remote repository."
