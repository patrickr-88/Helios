# Wireframes

Six views behind one persistent frame. The sidebar and title bar never move; the
tabs swap what fills the middle. Everything below is implemented — these are
annotations of the shipped interface, not proposals.

## Frame

```
┌──────────────┬──────────────────────────────────────────────────────────────────┐
│ ● ● ●        │  ┌────────────────────────────────────────┐  174 GB · 237 files  │
│              │  │Dashboard│Treemap│Folders│Largest│Cat.│Rep│   [Rescan] [Full]  │  ← title bar
│ VOLUMES      │  └────────────────────────────────────────┘                      │    (draggable)
│┌────────────┐│──────────────────────────────────────────────────────────────────│
││🖥 Macintosh││  ⟳ Scanning · 84,214 files · 41 GB  ▓▓▓▓▓▓▓░░░  about 2 min left │  ← only while
││  ▓▓▓░░░░░░ ││  …/Library/Caches/Google           [Pause] [Stop] │                │    scanning
││ 826 GB free││──────────────────────────────────────────────────────────────────│
│└────────────┘│  Macintosh HD › Users › alex › Movies                             │  ← breadcrumbs
│ REMOVABLE    │──────────────────────────────────────────────────────────────────│
│ 💾 Backup    │ [Search…] Size:▾ Modified:▾ Ext:[  ]      (Hidden)(System) (cats) │  ← filters
│  1.2 TB free │──────────────────────────────────────────────────────────────────│
│              │                                                                  │
│ OTHER        │                        « view content »                          │
│ 📂 Scan a    │                                                                  │
│    folder…   │                                                                  │
└──────────────┴──────────────────────────────────────────────────────────────────┘
                                                        ↑ a details panel slides in
                                                          from the right on selection
```

Sidebar volumes carry a usage meter that turns amber past 80% and red past 92% —
the one piece of alarm in the interface, and it earns its place.

## 1. Dashboard

```
┌────────────────────────────────────────────────────────────────────────────┐
│ ⚠ 2 locations could not be read.  [Show]                                   │
├──────────────┬──────────────┬──────────────┬───────────────────────────────┤
│ CAPACITY     │ USED         │ FREE         │ LAST SCAN                     │
│ 1.0 TB       │ 174 GB       │ 826 GB       │ just now                      │
│ APFS · /     │ ▓▓░░░░░ 17%  │ 83% available│ 237 files · 45 folders · 8.4 s │
├──────────────┴──────────────┴──────────────┴───────────────────────────────┤
│ Where the space went          by file category                             │
│   Documents  ▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░░░   46 GB   26%            │
│   Developer  ▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   39 GB   22%            │
│   Videos     ▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   35 GB   20%            │
├────────────────────────────────┬───────────────────────────────────────────┤
│ Largest folders                │ Largest files                             │
│  Library                 78 GB │  invoice-2.pdf                     12 GB   │
│  Applications            48 GB │  app-1.ts                          11 GB   │
└────────────────────────────────┴───────────────────────────────────────────┘
```

Opens on a cached snapshot at launch, so the first thing a user sees is data,
not a "Scan" button. The warning strip only exists when there is something to
warn about, and it says how much of the volume the scan actually covered.

## 2. Treemap

```
┌────────────────────────────────────────────────────────────────────────────┐
│┌─────────────────────┬──────────────┬────────────┬────────────────────────┐│
││ Movies              │ Photos       │ Xcode.app  │ node_modules           ││
││ 412 GB              │ ┌──────┬─────┤ 38 GB      │ 22 GB                  ││
││ ┌─────────┬────────┐│ │2023  │2024 │            │ ┌──────┬────┬────┬────┐││
││ │ raw     │ export ││ ├──────┴─────┤ (package:  │ │react │ts  │... │    │││
││ │ 210 GB  │ 140 GB ││ │  originals │  not       │ └──────┴────┴────┴────┘││
││ └─────────┴────────┘│ └────────────┤  expanded) │                        ││
│└─────────────────────┴──────────────┴────────────┴────────────────────────┘│
│                                     ┌──────────────────────────────┐        │
│                                     │ raw                          │ ← hover│
│                                     │ 210 GB · 51% of this folder  │        │
│                                     │ Double-click to open         │        │
│                                     └──────────────────────────────┘        │
├────────────────────────────────────────────────────────────────────────────┤
│ ■ Documents ■ Images ■ Videos ■ Audio ■ Archives ■ Apps ■ Dev ■ System      │
└────────────────────────────────────────────────────────────────────────────┘
```

Squarified layout computed in Rust, painted to one canvas. Colour is category;
depth is a light tint, so nesting reads without borders. Labels appear only
where they genuinely fit. Click selects, double-click drills in, breadcrumbs
climb back out. Bundles are drawn as one tile — a user thinks of Xcode as one
15 GB thing, not 40,000 files.

## 3. Folders

```
┌────────────────────────────────────────────────────────────────────────────┐
│ Folder                                    Size   Share of parent           │
├────────────────────────────────────────────────────────────────────────────┤
│ ▾ ■ Users                               412 GB   ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░   71%   │
│   ▾ ■ alex                              408 GB   ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░   99%   │
│     ▸ ■ Movies                          210 GB   ▓▓▓▓▓▓▓░░░░░░░░░░   51%   │
│     ▸ ■ Library            [partial]     94 GB   ▓▓▓░░░░░░░░░░░░░░   23%   │
│     ▸ ■ Downloads                        38 GB   ▓░░░░░░░░░░░░░░░░    9%   │
└────────────────────────────────────────────────────────────────────────────┘
```

Children are fetched per folder on first expand and cached. A folder Helios
could not fully read is badged `partial`, so a suspiciously small number is
explained rather than merely wrong.

## 4. Largest

```
┌────────────────────────────────────────────────────────────────────────────┐
│ (Files) (Folders)                                       Top 100 by size    │
├─────┬──────────────────────────────────┬─────────┬──────────┬──────┬───────┤
│  #  │ Path                             │    Size │ Share    │ Kind │ Mod.  │
├─────┼──────────────────────────────────┼─────────┼──────────┼──────┼───────┤
│  1  │ ■ /Users/alex/Movies/raw/a001.mov│   48 GB │ ▓▓▓▓▓▓▓▓ │Videos│ Mar 2 │
│  2  │ ■ /Users/alex/VMs/win11.qcow2    │   32 GB │ ▓▓▓▓▓░░░ │Other │ Jan 8 │
└─────┴──────────────────────────────────┴─────────┴──────────┴──────┴───────┘
```

Computed with a bounded heap, so "top 100 of 10 million" is one linear pass.
Rows are windowed — 100,000 results scroll like 20. Sortable by size, name or
date; the same table backs search results.

## 5. Categories

```
┌────────────────────────────────────────────────────────────────────────────┐
│ Storage by category      files only — folders are counted through contents │
├─────────────────┬────────────┬─────────┬───────────────────────────────────┤
│ ■ Videos        │    412 GB  │  1,284  │ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░  58.2%  │
│ ■ Images        │    118 GB  │ 42,910  │ ▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░  16.7%  │
│ ■ Applications  │     64 GB  │    312  │ ▓▓▓░░░░░░░░░░░░░░░░░░░░░░   9.0%  │
└─────────────────┴────────────┴─────────┴───────────────────────────────────┘
  Click a category to filter every other view by it.
```

Cross-filtering is the point: click Videos, switch to Treemap, and the map is
now only videos.

## 6. Reports

```
┌────────────────────────────────────────────────────────────────────────────┐
│ Export a report      read-only — nothing on the scanned volume is touched  │
│  Include the top [100 ▾] files and folders                                 │
│  ┌──────────────┬──────────────┬──────────────┐                            │
│  │ CSV          │ JSON         │ PDF          │                            │
│  │ Numbers/Excel│ For scripts  │ Printable    │                            │
│  │ [Export CSV] │ [Export JSON]│ [Export PDF] │                            │
│  └──────────────┴──────────────┴──────────────┘                            │
├────────────────────────────────────────────────────────────────────────────┤
│ Locations that could not be read                              2 shown      │
│  /Library/Application Support/MobileSync      permission denied            │
└────────────────────────────────────────────────────────────────────────────┘
```

## Details panel

```
┌──────────────────────────────┐
│ a001.mov                     │
│ /Users/alex/Movies/raw/a001…│
│ [Reveal in Finder]           │
│                              │
│ Size              48.2 GB    │
│ On disk           48.2 GB    │
│ Kind              Videos     │
│ Modified   Mar 2, 2026 14:03 │
│ Share of parent      23.0%   │
│ Share of scan         6.81%  │
└──────────────────────────────┘
```

Everything shown comes from metadata the scan already read — Helios never opens
a file to describe it. "Reveal in Finder" is the only outward action, and
deliberately so: finding the problem is this app's job, deleting it is Finder's.

## Interaction rules

| Gesture | Result |
|---|---|
| Click a volume | Load its cached scan instantly, or start one |
| Click a tile / row | Select — details panel slides in |
| Double-click a folder | Drill in (treemap, tables and breadcrumbs all follow) |
| Click a breadcrumb | Climb back to that level |
| Type in search | Debounced full-tree search; a `/` makes it a path search |
| Click a category | Filters every view |
| Rescan | Incremental — reuses unchanged folders |
| Full scan | Walks everything |

## Visual system

macOS-native without imitation: system font stack, Apple accent blue,
translucent sidebar, 6px radii, hairline separators, tabular numerals in every
size column. Light and dark are separate token sets rather than an inversion,
because inverted greys read as muddy against a real macOS dark desktop.

Responsive at three widths: full three-column layout, then the details panel
drops below 1080px, then the sidebar collapses to icons below 820px.
`prefers-reduced-motion` disables the progress sweep and the spinner.

## Accessibility

Semantic buttons and tables throughout, so VoiceOver reads rows as rows.
Category is never carried by colour alone — every swatch sits beside a label,
and the palette varies lightness as well as hue so adjacent categories stay
distinguishable under deuteranopia. Focus rings use the system accent at 2px.
The treemap is the one canvas-only view; its content is fully reachable through
the Folders and Largest views, which is the accessible path to the same data.
