# Security and privacy

Helios reads your entire disk. That is an unusual amount of trust to ask for, so
the design goal is that the trust should not be necessary: the app should be
structurally incapable of the things you would worry about.

## The four guarantees

### 1. It never modifies what it scans

No create, write, rename, delete, truncate, permission change or timestamp
change against a scanned volume — anywhere in the engine.

Enforced two ways in `crates/helios-core/tests/read_only.rs`:

- **Behaviourally.** A real scan runs over a fixture tree that is fingerprinted
  (size, mtime to the nanosecond, permissions) before and after. Any difference
  fails.
- **Structurally.** A source audit fails the build if `fs::write`,
  `fs::remove_*`, `fs::rename`, `fs::create_dir`, `OpenOptions`,
  `File::set_len`, `unlink(`, `chmod(` or `utimes(` appears anywhere in the
  engine outside the snapshot module.

The second check is the one that keeps holding after someone adds a feature the
first check does not happen to exercise.

Helios writes exactly two kinds of file, both outside any scanned tree: its
snapshot cache under the user's application-support directory, and reports to a
path the user picked in a save panel.

### 2. It never touches the network

There is no HTTP client, no socket, no update check, no crash reporter, no
analytics, no telemetry — not disabled by default, absent. A second source audit
fails the build if `TcpStream`, `UdpSocket`, `TcpListener`, `reqwest`, `hyper::`
or `ureq` appears in the engine.

The engine's entire dependency list is `serde`, `serde_json`,
`crossbeam-channel`, and `libc`/`windows-sys`. None performs I/O beyond the
standard library. The app adds Tauri and its dialog plugin — no HTTP plugin, no
updater plugin.

The app works with the Wi-Fi off, and that is the intended way to verify this
claim in about ten seconds. `tcpdump` or Little Snitch will confirm it more
rigorously.

### 3. Nothing leaves the machine

No cloud sync, no accounts, no file uploads, no "anonymous usage statistics",
no hashes of your filenames sent anywhere. Scan results live in memory and in a
local cache you can delete at any time (`Forget scan` in the UI, or
`rm -rf ~/Library/Application\ Support/Helios`).

### 4. It asks for no privilege it does not need

No `sudo`, no elevation prompt, no privileged helper tool, no kernel extension,
no raw device access. Helios runs as you and sees what you can see. Where macOS
denies a path, it records the denial and moves on — it never tries to work
around one.

## Attack surface

**Content Security Policy.** The webview's CSP is
`default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline';
script-src 'self'; connect-src 'self' ipc: http://ipc.localhost`. No remote
scripts, no remote styles, no remote images, no remote connections. The favicon
is an inline data URI so the app makes no external asset request at all.

**Capability allowlist.** `src-tauri/capabilities/default.json` grants the
renderer its own commands plus the save/open dialogs — and nothing else. There
is no filesystem plugin, no shell plugin, no HTTP plugin. If the renderer were
compromised, it could not read `~/Documents` or run a command, because the IPC
surface it can reach contains no way to do either.

**Untrusted input is filenames.** Everything the engine ingests is
attacker-influenceable in principle — a filename can contain quotes, newlines,
path separators, invalid UTF-8, 255 bytes of emoji. Handling:

| Vector | Handling |
|---|---|
| Names in CSV exports | RFC 4180 quoting; a name containing `",\n` cannot corrupt the file |
| Names in PDF exports | PDF string escaping; characters outside WinAnsi become `?` |
| Names in the UI | React escapes by default; no `dangerouslySetInnerHTML` anywhere |
| Invalid UTF-8 names | Lossy conversion at the boundary; never a panic |
| Volume ids as cache filenames | Sanitized to `[A-Za-z0-9_]`; a test asserts `../../etc/passwd` cannot escape the cache directory |
| Symlink loops | Links are never followed |
| Absurd path depth | Rollup is iterative; no recursion to overflow |

**Snapshot files are parsed defensively.** Length fields are range-checked
before any allocation (a corrupted header cannot become a multi-gigabyte
`Vec::with_capacity`), truncation is detected, unknown flag bits are dropped,
and a version mismatch is a discard rather than a best-effort parse. Snapshots
are a cache and are never treated as a source of truth.

**`unsafe` is confined.** Three places, all in `platform/`: `getfsstat` on
macOS, `statvfs` on Linux, and the Win32 volume calls. Each is a documented
two-call buffer-sizing pattern or a plain FFI call into an out-parameter we own.
The crate sets `#![forbid(unsafe_op_in_unsafe_fn)]`. Everything above the
platform layer is safe Rust.

**Reveal in Finder** is the one outward action, and it spawns
`open -R <path>` / `explorer /select,<path>` with the path as a separate
argument — no shell, no string interpolation into a command line.

## macOS permissions

Under the standard security model, an unprivileged app cannot read
`~/Library/Mail`, Photos libraries, Messages data, `~/Desktop`, `~/Documents` or
`~/Downloads` without user consent. Helios's behaviour:

- Denials are recorded per path with the reason, and surfaced in the UI and in
  every report.
- Folders it could not fully read are flagged, and their sizes are presented as
  lower bounds rather than totals.
- The dashboard compares scanned bytes to the volume's used bytes and says so
  when coverage is short.
- The inspector explains that Full Disk Access (System Settings → Privacy &
  Security) would let a rescan see the rest.

It never demands the permission, degrades into uselessness without it, or nags.

## Distribution

Hardened runtime, notarized, signed `.dmg`. No entitlements beyond a read-only
user-space app's needs. Builds are reproducible from the repository, and the
engine is small enough to actually read: roughly 3,000 lines of Rust, three
dependencies, no network code.

## Reporting a vulnerability

Open a GitHub issue for anything non-sensitive. For something exploitable,
please use GitHub's private security advisory flow rather than a public issue.

## Threat model, stated plainly

**In scope:** Helios corrupting or deleting user data (prevented structurally);
Helios leaking filesystem contents over a network (no network code); a
compromised renderer escalating to filesystem access (capability allowlist);
malicious filenames corrupting exports or crashing the app (escaping, defensive
parsing, no recursion).

**Out of scope:** an attacker who already has code execution as the user — at
that point they can read the disk directly and do not need Helios; other
applications reading the snapshot cache, which is protected by ordinary file
permissions and contains only metadata (names, sizes, dates), never file
contents.
