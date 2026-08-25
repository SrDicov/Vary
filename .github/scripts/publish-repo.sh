#!/bin/sh
# Build the signed rolling XBPS repository under repo/.
# Input: pkgs/*.xbps (both libcs) + XBPSSIGN_PRIVKEY env (PEM).
# Output: repo/{*.xbps,*.sig2,*-repodata,sha256sums.txt}
# Usage (cwd = repo root):  sh .github/scripts/publish-repo.sh
set -eu

REPO="https://repo-default.voidlinux.org/current"
mkdir -p /etc/xbps.d
echo "repository=$REPO" > /etc/xbps.d/00-repository-main.conf
xbps-install -S >/dev/null 2>&1
# glibc image ships ancient xbps; no-op on musl.
xbps-install -u xbps -y >/dev/null 2>&1
xbps-install -y openssl >/dev/null

[ -n "${XBPSSIGN_PRIVKEY:-}" ] || {
    echo "XBPSSIGN_PRIVKEY no está definido" >&2
    exit 1
}
umask 077
printf '%s\n' "$XBPSSIGN_PRIVKEY" > /tmp/priv.pem

SIGNER="Dicov (SrDicov) <https://github.com/SrDicov>"
mkdir -p repo
cp pkgs/*.xbps repo/
cd repo
# rindex matches packages against the host arch unless overridden, so index
# and sign each architecture explicitly.
for ARCH in x86_64 x86_64-musl; do
    pkgs="$(ls vary-*.${ARCH}.xbps 2>/dev/null || true)"
    [ -n "$pkgs" ] || continue
    XBPS_ARCH="$ARCH" xbps-rindex --signedby "$SIGNER" -a $pkgs
    XBPS_ARCH="$ARCH" xbps-rindex --signedby "$SIGNER" -s --privkey /tmp/priv.pem .
    for f in $pkgs; do
        XBPS_ARCH="$ARCH" xbps-rindex --signedby "$SIGNER" -S --privkey /tmp/priv.pem "$f"
    done
done
rm -f /tmp/priv.pem
sha256sum *.xbps *.sig2 *-repodata > sha256sums.txt
ls -la
