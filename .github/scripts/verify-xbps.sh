#!/bin/sh
# Install the built .xbps into a fresh Void Linux container and smoke-test it.
# Proves: package metadata is valid, the 'git' dependency resolves from the
# official repo, and the binary runs on the target libc.
# Usage (cwd = repo root):  sh .github/scripts/verify-xbps.sh glibc|musl
set -eu

LIBC="${1:?usage: verify-xbps.sh glibc|musl}"
REPO="https://repo-default.voidlinux.org/current"
[ "$LIBC" = "musl" ] && REPO="$REPO/musl"

mkdir -p /etc/xbps.d
echo "repository=$REPO" > /etc/xbps.d/00-repository-main.conf

xbps-install -S

cd pkgs
ls -lh
xbps-rindex -a vary-*.xbps
# -y because there is no TTY; git is pulled automatically as a dependency.
xbps-install -Sy --repository="$PWD" vary

# --- Functional smoke tests -------------------------------------------------
xbps-query vary | head -5

out="$(vary -V)"
echo "vary -V -> $out"
printf '%s\n' "$out" | grep -Eq '^vary [0-9]+\.[0-9]+\.[0-9]+$'

vary -h > /dev/null
vary --repo list

echo "OK: vary .xbps ($LIBC) installed and functional"
