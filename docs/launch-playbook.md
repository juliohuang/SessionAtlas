# GitHub launch playbook

This playbook separates repository readiness from public promotion. Do not drive traffic to the project until the exact public commit, security settings, and downloadable beta have been verified.

## Positioning

One-line promise:

> Find every local AI coding project, remember which CLI touched it, and resume work from one local-first desktop console.

Lead with the problem and the working screenshot. Avoid claims such as “all AI tools,” “zero risk,” “production ready,” or “private by definition.” State that the Windows desktop build is beta, the CLI remains local, and third-party AI tools keep their own network behavior.

Before launch, make an explicit keep-or-rename decision for `SessionAtlas`. An unrelated commercial service currently uses `sessionatlas.nl`; the repository disclaimer reduces confusion but does not resolve trademark risk.

## GitHub settings checklist

- Make the repository public only after the final secret/history scan.
- Set the description to: `Local-first workspace for finding and resuming projects across Claude Code, Codex, Kimi, OpenCode, and Aider.`
- Add topics: `ai-cli`, `developer-tools`, `tauri`, `dotnet`, `rust`, `terminal`, `claude-code`, `codex`, `local-first`.
- Use `docs/images/sessionatlas-browser-demo.png` or a purpose-built 1280×640 derivative as the social preview.
- Enable Issues and Discussions; seed a welcome/roadmap discussion.
- Enable private vulnerability reporting, secret scanning, push protection, Dependabot alerts, dependency graph, and CodeQL.
- Protect `main`: require a pull request, CI and Security checks, conversation resolution, and no force pushes or deletions.
- Verify the Apache-2.0 license is detected and the community profile recognizes the contribution files.
- Publish `v0.1.0-beta.1` with MSI, NSIS installer, SHA-256 checksums, SPDX SBOM, provenance attestation, and honest unsigned-beta notes.
- Pin the beta release, roadmap discussion, and two newcomer-friendly issues.

## Launch sequence

### Seven days before

- Ask 5–10 users of at least two supported CLIs to try the isolated beta.
- Turn every confusing first-run step into documentation or an issue.
- Capture a 20–40 second silent GIF/video showing scan → search → resume. Use synthetic project names only.
- Label 3–5 small issues `good first issue` and add acceptance criteria.

### Launch day

- Publish the GitHub prerelease and verify a clean Windows installation.
- Post one clear demo to the maintainer's existing developer network first.
- Share adapted versions on communities where the maintainer already participates; follow each community's self-promotion rules.
- Be available for the first few hours to answer installation and privacy questions.

### First month

- Week 1: fix install and first-run defects before adding features.
- Week 2: publish a short architecture article explaining the local SQLite index and structured process boundary.
- Week 3: highlight a contributor or supported-tool workflow; avoid artificial star campaigns.
- Week 4: publish a transparent beta retrospective and update the roadmap from evidence.

## Reusable launch copy

### Short post

> I built SessionAtlas, a local-first workspace for people who use several AI coding CLIs. It scans local Claude Code, Codex, Kimi, OpenCode, and Aider histories, deduplicates projects, and lets you resume work from a Tauri desktop console with real PTY terminals. The first Windows beta, source, threat boundaries, SBOM, and checksums are on GitHub. Feedback on first-run setup and scanner compatibility is especially useful.

### Show HN title

> Show HN: SessionAtlas – a local-first workspace for projects across AI coding CLIs

### Discussion prompt

> Which two or more AI coding CLIs do you switch between, and what context do you most often lose when moving from one to another? Please do not share private project names or session content.

## Measure useful impact

Review weekly, not hourly:

- unique visitors → README/release visitors → installer downloads;
- successful first scans and supported-tool compatibility reports (only from voluntary issue/discussion feedback; no product telemetry);
- time to first maintainer response and time to close install blockers;
- returning contributors, accepted pull requests, and contributor retention;
- stars and forks as secondary discovery indicators, never as the primary product goal.

Record qualitative feedback alongside counts. A smaller group that installs, scans successfully, and contributes evidence is more valuable than unexplained traffic.
