#!/bin/bash
set -euo pipefail

# Release script for rpm-qa crate
# Usage: ./tools/release.sh [--dry-run]

DRY_RUN=false

for arg in "$@"; do
    case "$arg" in
        --dry-run)
            DRY_RUN=true
            ;;
        *)
            echo "Unknown argument: $arg" >&2
            echo "Usage: $0 [--dry-run]" >&2
            exit 1
            ;;
    esac
done

# Extract version from Cargo.toml
VERSION=$(cargo metadata --format-version=1 --no-deps | jq -r '.packages[0].version')
if [[ -z "$VERSION" || "$VERSION" == "null" ]]; then
    echo "Failed to extract version from Cargo.toml" >&2
    exit 1
fi
TAG="v${VERSION}"

echo "Releasing ${TAG}"

# Check for uncommitted tracked changes
if ! git diff --quiet HEAD; then
    echo "Error: There are uncommitted changes." >&2
    exit 1
fi

# Run tests
echo "Running cargo test..."
cargo test

# Build cargo publish arguments
PUBLISH_ARGS=()
if [[ "$DRY_RUN" == "true" ]]; then
    PUBLISH_ARGS+=(--dry-run)
fi

# Publish to crates.io
echo "Running cargo publish ${PUBLISH_ARGS[*]:-}..."
cargo publish "${PUBLISH_ARGS[@]}"

# Skip tag creation in dry-run mode
if [[ "$DRY_RUN" == "true" ]]; then
    echo "Dry run complete. Skipping tag creation and push."
    exit 0
fi

# Create and push tag
if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "Tag $TAG already exists"
else
    echo "Creating tag $TAG..."
    git tag -s -a "$TAG" -m "Release $TAG"
fi

echo "Tag $TAG ready to be pushed!"
