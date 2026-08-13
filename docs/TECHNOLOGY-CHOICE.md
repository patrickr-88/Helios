# Technology choice

**Recommendation: Tauri + React + TypeScript, with the scan engine as a
standalone Rust crate that none of the above can see.**

The important half of that sentence is the second half. The engine is a library
with no knowledge of Tauri, webviews or JavaScript; the shell that binds it to a
UI is ~450 lines. That structure is what makes the shell choice reversible — if
Tauri turns out to be wrong in two years, the thing that took months to get
right is untouched.

## The three options, measured against what this app actually is

Helios is a CPU- and syscall-heavy data tool with a fairly simple interface. It
has to scan millions of files without heating the machine, hold a large tree in
a modest memory budget, and ship as something a privacy-minded user will trust.
That profile pushes hard on runtime overhead and hardly at all on UI framework
richness.

| | **Tauri + React** | **Electron + React** | **Native SwiftUI** |
|---|---|---|---|
| Installed size | ~8–12 MB | ~150–200 MB | ~5 MB |
| Idle RSS (empty window) | ~60–90 MB | ~200–300 MB | ~40 MB |
| Scan engine language | Rust | Node/N-API or a Rust sidecar | Swift |
| Windows port | Recompile + one platform module | Same | **Full rewrite** |
| macOS look and feel | Good with care | Good with care | Perfect, free |
| Sandboxing / permissions | Per-command capability allowlist | All-or-nothing Node access | App Sandbox + entitlements |
| Attack surface | Rust core + system WebView | Full Chromium + Node runtime | Cocoa |
| Team can hire for it | Web + some Rust | Web | Swift only |
| Build/release complexity | Moderate (cross-compile, signing) | Low | Low on Apple, N/A elsewhere |

### Electron

Rejected primarily on the engine, not the bundle size. Node cannot walk a
filesystem at the speed this app needs — `fs.readdir`/`fs.lstat` through libuv's
thread pool, with a V8 object per entry, is several times slower than a native
walker and produces garbage-collected objects where we need a packed arena. The
realistic Electron design is *"a Rust sidecar binary plus Electron"*, at which
point you are running Chromium and Node purely to host a window, and paying
150+ MB and ~250 MB of RSS for it.

For an app whose pitch is "small, fast, private, offline", shipping a full
browser engine and a JavaScript runtime with unrestricted filesystem access is
also the wrong story to tell users.

Electron would win if Helios needed heavy web-platform features (rich text,
video, an ecosystem of Chromium-only APIs) or a large existing web codebase. It
needs neither.

### Native SwiftUI

The best macOS app of the three, and the reason it is not the answer is
`Deliverable 10: future Windows support`. SwiftUI on Windows is not a thing;
choosing it means writing the product twice and maintaining two implementations
of the same scanning subtleties — hardlinks, sparse files, symlink loops,
permission handling — forever. That is the most expensive possible way to reach
Windows.

There is also a subtler cost: Swift's filesystem APIs (`FileManager`,
`URLResourceValues`) allocate per entry and are noticeably slower than raw
`readdir`/`lstat`, so a fast Swift scanner ends up calling POSIX directly
anyway — writing C-flavoured Swift to match what the Rust engine does natively.

SwiftUI is the right call if macOS is the only target that will ever matter, or
if the app needs deep system integration (Finder extensions, Quick Look
providers, App Store distribution with the sandbox). Worth revisiting for a
Helios *menu bar* companion, where native fit matters most and the scanning
lives in the shared engine anyway.

### Tauri

Wins on the axes that matter here:

- **The engine is Rust, natively.** No FFI boundary in the hot path, no sidecar
  process, no serialization tax on the scan itself. Millions of `lstat` calls
  and a 56-byte-per-node arena are things Rust does without ceremony.
- **The system WebView.** WebKit on macOS, WebView2 on Windows. The app ships
  ~10 MB instead of ~180 MB, and gets OS security updates for its renderer.
- **Capability-based permissions.** Helios grants its webview exactly its own
  commands plus the save/open dialogs — no filesystem plugin, no shell plugin,
  no HTTP plugin. A compromised renderer cannot read `~/Documents`, because the
  IPC surface it can reach does not include a way to.
- **One UI, both platforms.** The React interface compiles unchanged for
  Windows; only `platform/windows.rs` needs finishing.

The honest costs, stated plainly:

- **Two WebView engines to test.** WebKit and WebView2 differ, mostly in CSS
  edge cases. Mitigated by keeping the UI plain — one stylesheet, no exotic
  layout, canvas for the only heavy view.
- **Smaller ecosystem than Electron.** Fewer recipes, occasional rough edges in
  bundling and signing. Tauri 2 is stable enough for a tool this size, and
  nothing here depends on an exotic plugin.
- **Rust in the team's stack.** Real, and appropriate: the hard part of this app
  *is* systems programming, and would be systems programming in any language.
- **Native chrome takes effort.** A web UI is macOS-shaped only if you make it
  so — hence the system font stack, transparent title bar, real dark mode
  tokens, and a canvas treemap rather than thousands of DOM nodes.

## What the structure buys, regardless

Because `helios-core` is a plain library with no shell dependency:

- The **CLI** is a first-class front end (and how the engine is benchmarked).
- **Tests run headless** on Linux CI with no display, no webview, no bundling.
- A **SwiftUI or WinUI front end** remains possible later without touching the
  engine — the same crate would back it.
- The **shell is disposable**. That is the point: the expensive, subtle,
  correctness-critical code is behind a boundary that no UI decision can reach.

## Verdict

Ship Tauri + React + TypeScript. Keep the engine a standalone crate, keep the
shell thin enough to rewrite in an afternoon, and keep every OS-specific line
behind `platform/`. That combination gets a small fast private app on macOS now,
a Windows build for the cost of one module, and no lock-in to the framework
choice if the landscape shifts.
