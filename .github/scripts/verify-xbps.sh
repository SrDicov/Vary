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
# The glibc image ships an ancient xbps that refuses to install anything
# until itself is updated; on musl this is a no-op.
xbps-install -u xbps -y

ls -lh pkgs
xbps-rindex -a pkgs/vary-*.xbps
# 'git' is pulled automatically as a versioned dependency of vary.
xbps-install -y --repository="$PWD/pkgs" vary

# --- Functional smoke tests -------------------------------------------------
xbps-query vary | head -5

out="$(vary -V)"
echo "vary -V -> $out"
printf '%s\n' "$out" | grep -Eq '^vary [0-9]+\.[0-9]+\.[0-9]+$'

vary -h > /dev/null
vary --repo list

echo "OK: vary .xbps ($LIBC) installed and functional"
