# Technology choices

**Helios is a Rust command-line program with a library at its core.** It was
briefly a desktop app; that layer was built, worked, and then removed. This
document records both decisions, because the second one is only defensible if
you can see what it cost.

## Why Rust

The hard part of this product is systems programming: millions of `lstat` calls,
a 56-byte-per-node arena that has to fit ten million entries in a sane memory
budget, a parallel walker that must not deadlock, and platform APIs
(`getfsstat`, `GetLogicalDrives`) called directly.

That rules out the scripting languages on throughput and memory layout grounds —
Node's `fs.readdir`/`fs.lstat` through libuv, with a garbage-collected object per
entry, is several times slower and cannot produce a packed arena at all. It
leaves C, C++, Go, Swift and Rust. Rust wins on three specifics that matter here:

- **No garbage collector**, so a 700 MB arena is 700 MB, not 700 MB plus
  collector headroom and pauses mid-scan.
- **Memory safety across a threaded walker**, which is exactly where a C
  implementation of this design would spend its bug budget.
- **One binary, no runtime**, which is what makes the flash-drive story work:
  756 KB that runs on any Mac without installing anything.

Go would be a reasonable second choice and would cost perhaps 2× the memory per
node with a GC in the loop. Swift would tie the engine to Apple platforms, which
the Windows requirement rules out.

## Why a command-line program

Because it is the whole product, at a twentieth of the weight.

The questions a disk tool answers — what is big, what is where, what kind of
thing is it — are answered by ranked lists, proportional bars and an indented
tree. A terminal renders all of those. What a GUI genuinely adds is the treemap:
an at-a-glance shape of a disk that no text view reproduces. That is one view,
and it cost a webview, a JavaScript toolchain, an IPC bridge and a bundling
pipeline.

What removing it bought:

| | Desktop app | Command-line program |
|---|---|---|
| Ships as | ~10 MB `.app` bundle + `.dmg` | **756 KB binary** |
| To build | Rust **and** Node 20+, npm install, Vite, Tauri CLI | **Rust** |
| To install | Build, sign, notarize, drag to /Applications | Copy one file |
| Runtime | System WebView process + Rust process | **One process, 2 ms startup** |
| Attack surface | Webview, CSP, IPC command allowlist | Reads directories; writes one cache file |
| Lines of code | ~5,500 (Rust + TypeScript + CSS) | **~3,500 Rust** |
| Composes with other tools | No | `helios / --find x \| grep`, cron, scripts |

The engine did not change. It was a library the shell called, and it is now a
library the program calls — which is why deleting a whole front end was a day's
work rather than a rewrite, and why the "what if we want the GUI back" answer is
"add a second front end", not "start over".

## The comparison that was made when there was a GUI

Kept because it is still the right analysis *if* a graphical version returns, and
because the reasoning explains the shape of the code that survived.

| | **Tauri + React** | **Electron + React** | **Native SwiftUI** |
|---|---|---|---|
| Installed size | ~8–12 MB | ~150–200 MB | ~5 MB |
| Idle RSS | ~60–90 MB | ~200–300 MB | ~40 MB |
| Scan engine language | Rust | Node, or a Rust sidecar | Swift |
| Windows port | Recompile + one platform module | Same | **Full rewrite** |
| macOS look and feel | Good with care | Good with care | Perfect, free |
| Attack surface | Rust core + system WebView | Full Chromium + Node | Cocoa |

**Electron** was rejected on the engine, not the bundle size: the honest Electron
design is "a Rust sidecar plus Electron", at which point you ship Chromium and a
JavaScript runtime purely to host a window.

**SwiftUI** would have made the best Mac app and failed the Windows requirement
outright — it would mean writing the scanning subtleties (hardlinks, sparse
files, symlink loops, permissions) twice, forever.

**Tauri** won that comparison, and the implementation confirmed the analysis: the
shell was ~450 lines, the UI ~1,500, and the whole thing sat on the same engine
the CLI uses. It was removed for weight, not because it was the wrong choice for
a GUI.

## Why the dependency list is four crates

`crossbeam-channel` for the walker's queues, `serde` and `serde_json` for the
JSON report, `libc`/`windows-sys` for syscalls. Everything else — argument
parsing, flag sets, the snapshot codec, the PDF writer, date and byte formatting
— is in-tree, each under 250 lines.

This is not dependency asceticism for its own sake. The product's claim is that
it reads your entire disk and can be trusted to do nothing else, and that claim
is only checkable if a person can actually read the code. `clap` is excellent
software; it is also more code than this entire program, to parse fourteen flags.

The one dependency that could plausibly go is `serde`, whose derive macro brings
`syn`, `quote` and `proc-macro2` — most of the dependency graph and most of the
build time. It stays because correct JSON escaping is easy to get subtly wrong
and the derive keeps the report's fields and its serialized shape in step.

## What would change these decisions

- **A treemap becomes essential.** Bring back a front end — Tauri by the
  comparison above, or SwiftUI if macOS becomes the only target. The engine is
  ready; `git log` has the deleted implementation.
- **Non-technical users become the audience.** A terminal is a real barrier. That
  is a GUI, and the analysis above applies.
- **Scans need to run somewhere without a filesystem to walk.** Not a thing.
