#!/usr/bin/env bash

set -euo pipefail

repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_dir"

for tool in cargo rustc rpmbuild; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Missing required tool on PATH: $tool" >&2
        exit 1
    fi
done

name="$(awk -F ' *= *' '$1 == "name" {gsub(/"/, "", $2); print $2; exit}' Cargo.toml)"
version="$(awk -F ' *= *' '$1 == "version" {gsub(/"/, "", $2); print $2; exit}' Cargo.toml)"
topdir="${RPM_TOPDIR:-$repo_dir/target/rpmbuild}"
stage_dir="$(mktemp -d)"
trap 'rm -rf -- "$stage_dir"' EXIT

mkdir -p "$topdir/BUILD" "$topdir/BUILDROOT" "$topdir/RPMS" "$topdir/SOURCES" "$topdir/SPECS" "$topdir/SRPMS"
mkdir -p "$stage_dir/$name-$version"

tar --exclude=.git --exclude=target -cf - . | tar -C "$stage_dir/$name-$version" -xf -
mkdir -p "$stage_dir/$name-$version/.cargo"
(
    cd "$stage_dir/$name-$version"
    cargo vendor --locked --versioned-dirs vendor >.cargo/config.toml
)
tar -C "$stage_dir" -czf "$topdir/SOURCES/$name-$version.tar.gz" "$name-$version"
cp packaging/asus-tablet-switch.spec "$topdir/SPECS/"

# A developer may use a rustup/mise compiler that RPM's package database cannot
# see. The spec retains its real BuildRequires for Fedora builders; this local
# helper verifies the commands above and skips only RPM's dependency preflight.
rpmbuild -ba --nodeps --define "_topdir $topdir" "$topdir/SPECS/asus-tablet-switch.spec"
