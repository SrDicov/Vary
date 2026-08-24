#!/bin/sh
# Build an .xbps binary package for vary inside a Void Linux container.
# Usage (cwd = repo root):  sh .github/scripts/build-xbps.sh glibc|musl
set -eu

LIBC="${1:?usage: build-xbps.sh glibc|musl}"
REPO="https://repo-default.voidlinux.org/current"

case "$LIBC" in
    glibc) ARCH="x86_64" ;;
    musl)  ARCH="x86_64-musl"; REPO="$REPO/musl" ;;
    *) echo "unknown libc: $LIBC" >&2; exit 1 ;;
esac

# The glibc image points at alpha.de.repo.voidlinux.org whose TLS cert
# mismatches; repo-default works everywhere (musl image already defaults to it).
mkdir -p /etc/xbps.d
echo "repository=$REPO" > /etc/xbps.d/00-repository-main.conf

xbps-install -S
# The glibc image ships an ancient xbps that refuses to install anything
# until itself is updated; on musl this is a no-op.
xbps-install -u xbps -y
xbps-install -S
xbps-install -y rust cargo git

VER="$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)"
PKGVER="vary-${VER}_1"

nproc
rustc --version

cargo build --release --jobs "$(nproc)"
./target/release/vary -V

rm -rf dest
mkdir -p dest/usr/bin dest/usr/share/doc/vary dest/usr/share/licenses/vary
cp target/release/vary dest/usr/bin/vary
cp README.md docs/VURINFO.md docs/AUR2XBPS.md etc/vary.conf.example dest/usr/share/doc/vary/
cp LICENSE dest/usr/share/licenses/vary/

rm -f "${PKGVER}.${ARCH}.xbps"
xbps-create -A "$ARCH" -n "$PKGVER" \
    -s "Void User Repository (VUR) helper and build automator" \
    -m "Dicov (SrDicov) <https://github.com/SrDicov>" \
    -l "GPL-3.0" \
    -H "https://github.com/SrDicov/Vary" \
    -D "git" \
    dest
printf '%s\n' "$VER" > version.txt
ls -lh "${PKGVER}.${ARCH}.xbps" version.txt
