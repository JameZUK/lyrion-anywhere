# Lyrion Anywhere — working rules

The design is in `SPEC.md`. Keep it current when decisions change.

## Standing rule: no personal or homelab information in this repo

This repository is public. Never commit personal or environment-specific information. That includes:
- IP addresses
- hostnames and domains
- server or player names
- library details and filesystem paths from a real install
- credentials, tokens and email addresses

This applies to code, tests, fixtures, docs, commit messages and PR descriptions.

- Use placeholders in anything committed: `lms.example.lan`, `10.0.0.10`, `music.example.com`, `/srv/music`.
- Real values go only in gitignored local files: `.env.local`, `dev/local.*`.
- Before every commit, scan the staged diff for addresses, hostnames and names.

## Standing rule: tests at every point, security first

Every change ships with tests for security, stability and reliability. Security is the top priority. No component, endpoint or parser lands without tests.

**Security**
- Every externally reachable surface has negative tests that prove attacks are refused:
  - parsers, the gateway endpoints and auth;
  - the RPC allow-list, path and track resolution, the Slimproto HELO/strm/serv enforcement;
  - the plugin settings handlers.
- Any parser of untrusted bytes also gets fuzz/robustness tests, which must never panic.
- Security fixes get a regression test that fails before the fix.

**Stability**
- Unit tests for state machines and edge cases: truncated or oversize frames, disconnects, rate changes.
- Robustness tests on arbitrary input.

**Reliability**
- Integration tests against fake LMS and fake clients, covering reconnect/resume, back-pressure and timeouts.
- Opt-in live tests against a real LMS (dedicated muted test player, config from gitignored `dev/local.env`).

All tests run in CI on every push. Don't call work done while any tests are failing or skipped.

## Conventions

- British English in docs and UI strings.
- Clean-room implementation: squeezelite (GPL-3.0) and Material Skin are references for behaviour only. Never copy their code.
- Rust core (`core/`) does no I/O. Hosts (native CLI, WASM, later mobile) drive the network and audio output.
- Tests run against a live LMS must use a dedicated test player and leave other players untouched.
