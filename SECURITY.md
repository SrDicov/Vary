# Security Policy

## Supported versions

Only the latest commit on `vary-mvp` receives security fixes during the MVP phase.

## Reporting a vulnerability

**Do not open a public issue for security vulnerabilities.**

Use GitHub's private vulnerability reporting at <https://github.com/SrDicov/Vary/security/advisories>, or contact [SrDicov](https://github.com/SrDicov) directly.

## Scope notes

- vary runs external commands (`git`, `xbps-*`, `xbps-src`) and writes system files under `/etc/xbps.d/` via `sudo`. Reports involving command injection, key substitution, or unsafe template handling are high priority.
- VUR repos are third-party content: `.VURINFO` indexes are parsed as JSON (never executed). Any path where repo content leads to shell execution outside the `xbps-src` masterdir is a security bug.
