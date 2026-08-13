# Security and privacy

Helios reads your entire disk. That is an unusual amount of trust to ask for, so
the design goal is that the trust should not be necessary: the program should be
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

The entire dependency list is `serde`, `serde_json`, `crossbeam-channel`, and
`libc`/`windows-sys`. None performs I/O beyond the standard library.

It works with the Wi-Fi off, and that is the intended way to verify this claim in
about ten seconds. `tcpdump` or Little Snitch will confirm it more rigorously.

### 3. Nothing leaves the machine

No cloud sync, no accounts, no file uploads, no "anonymous usage statistics",
no hashes of your filenames sent anywhere. Scan results live in memory and in a
local cache you can delete at any time
(`rm -rf ~/Library/Application\ Support/Helios`).

Running from a flash drive moves even that cache off the host: see
[PORTABLE.md](PORTABLE.md). Two things worth knowing before you carry one
around. The drive then holds file and folder *names*, sizes and dates for every
machine scanned — never file contents, but a directory listing is not nothing,
so encrypt the drive if it will leave your control. And portable mode is about
Helios not storing your data on someone else's machine; the OS still records
that an app was launched, so it is not an anti-forensics tool and should not be
described as one.

### 4. It asks for no privilege it does not need

No `sudo`, no elevation prompt, no privileged helper tool, no kernel extension,
no raw device access. Helios runs as you and sees what you can see. Where macOS
denies a path, it records the denial and moves on — it never tries to work
around one.

## Attack surface

**There is almost none.** Helios is one process with no network code, no IPC, no
plugin loading, no scripting, and no privileged component. It reads directories
and writes one cache file. Removing the graphical front end deleted the only
parts with a meaningful attack surface: a webview, a content security policy, an
IPC command allowlist and a JavaScript dependency tree.

**Untrusted input is filenames.** Everything the engine ingests is
attacker-influenceable in principle — a filename can contain quotes, newlines,
path separators, invalid UTF-8, 255 bytes of emoji. Handling:

| Vector | Handling |
|---|---|
| Names in CSV exports | RFC 4180 quoting; a name containing `",\n` cannot corrupt the file |
| Names in PDF exports | PDF string escaping; characters outside WinAnsi become `?` |
| Names in terminal output | Truncated to a column width; control characters are passed through as the terminal's business, exactly as `ls` does |
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

**Helios spawns no processes at all.** The graphical version had one outward
action — "Reveal in Finder" — and it went with the interface. Nothing here
executes anything.

## macOS permissions

Under the standard security model, an unprivileged app cannot read
`~/Library/Mail`, Photos libraries, Messages data, `~/Desktop`, `~/Documents` or
`~/Downloads` without user consent. Helios's behaviour:

- Denials are recorded per path with the reason, and printed after the scan and
  in every report.
- Folders it could not fully read are flagged, and their sizes are presented as
  lower bounds rather than totals.
- The dashboard compares scanned bytes to the volume's used bytes and says so
  when coverage is short.
- [MACOS.md](MACOS.md) explains how to grant Full Disk Access if you want the
  rest.

It never demands the permission, degrades into uselessness without it, or nags.

## Distribution

One binary, built from source with `cargo build --release`. Nothing to sign or
notarize, because there is no bundle — and nothing to install, so there is no
installer to trust. The whole program is roughly 3,500 lines of Rust with four
dependencies and no network code, which is small enough that reading it is a
realistic afternoon rather than a figure of speech.

## Reporting a vulnerability

Open a GitHub issue for anything non-sensitive. For something exploitable,
please use GitHub's private security advisory flow rather than a public issue.

## Threat model, stated plainly

**In scope:** Helios corrupting or deleting user data (prevented structurally);
Helios leaking filesystem contents over a network (no network code); malicious
filenames corrupting exports or crashing the program (escaping, defensive
parsing, no recursion).

**Out of scope:** an attacker who already has code execution as the user — at
that point they can read the disk directly and do not need Helios; other
applications reading the snapshot cache, which is protected by ordinary file
permissions and contains only metadata (names, sizes, dates), never file
contents.
