# Security Policy

## Supported versions

OpenStrata is pre-1.0 and ships from a single active line. Security fixes land on
`main` and in the next tagged release; only the latest release (`v*`) is
supported. There are no long-term-support branches yet.

| Version | Supported |
| --- | --- |
| latest release (`main`) | ✅ |
| older releases | ❌ (upgrade to latest) |

## Reporting a vulnerability

Please report suspected vulnerabilities privately rather than opening a public
issue.

- Email: **piggypenguin583@gmail.com**
- Alternatively, use GitHub's private
  [Security Advisories](https://github.com/animu-sphere/open-strata/security/advisories/new)
  ("Report a vulnerability") for the repository.

Please include, where possible:

- the affected version, commit, or artifact digest;
- your platform (OS / arch) and how the runtime was obtained (mock / adopted /
  built / pulled by digest);
- a description of the issue and its impact;
- reproduction steps or a proof of concept;
- any suggested remediation.

Do not include third-party secrets or live credentials in a report.

## What to expect

This is a small project maintained on a best-effort basis. We aim to:

- acknowledge a report within a few business days;
- confirm the issue and assess severity;
- keep you updated as a fix is developed;
- credit reporters who wish to be named once a fix ships.

Please give us reasonable time to release a fix before any public disclosure. We
support coordinated disclosure via GitHub Security Advisories.

## Scope

In scope: the `ost` CLI and its libraries, the artifact/runtime transport and
verification path, packaging and extraction, plugin bundle loading, generated CI
workflows, and the release/distribution path.

Out of scope: vulnerabilities in third-party components OpenStrata builds, links,
or distributes (OpenUSD, MaterialX, their transitive dependencies, and host
toolchains) — report those upstream, though we welcome a heads-up if OpenStrata's
handling of them makes an issue materially worse.

## Hardening status

The security baseline landed so far — packaging that rejects symlinks and special
files, bundle-relative path enforcement, atomic `O_EXCL` writes, SHA-pinned CI
Actions, and SLSA build-provenance attestations on release artifacts — and the
remaining work (installer/asset signature verification and a runtime trust policy)
are tracked in the
[roadmap security baseline](docs/roadmap/README.md#security-baseline).
