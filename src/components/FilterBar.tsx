/**
 * Search and filtering.
 *
 * Text input is debounced by the caller; everything here is a controlled
 * component over a single `Filter` object, which is the same shape the engine
 * takes, so there is no translation layer to drift.
 */

import { CATEGORY_LABELS } from "../lib/format";
import { CATEGORIES } from "../lib/types";
import type { Category, Filter } from "../lib/types";

interface Props {
  filter: Filter;
  onChange: (filter: Filter) => void;
  /** Search text is held separately so typing stays responsive. */
  query: string;
  onQueryChange: (query: string) => void;
}

const SIZE_STEPS: Array<[string, number | null]> = [
  ["Any size", null],
  ["> 1 MB", 1_000_000],
  ["> 10 MB", 10_000_000],
  ["> 100 MB", 100_000_000],
  ["> 1 GB", 1_000_000_000],
];

const AGE_STEPS: Array<[string, number | null]> = [
  ["Any date", null],
  ["Last 7 days", 7],
  ["Last 30 days", 30],
  ["Last year", 365],
  ["Older than a year", -365],
];

export function FilterBar({ filter, onChange, query, onQueryChange }: Props) {
  const set = (patch: Partial<Filter>) => onChange({ ...filter, ...patch });

  const toggleCategory = (category: Category) => {
    const current = filter.categories ?? [];
    const next = current.includes(category)
      ? current.filter((c) => c !== category)
      : [...current, category];
    set({ categories: next });
  };

  const applyAge = (days: number | null) => {
    if (days == null) {
      set({ modifiedAfter: null, modifiedBefore: null });
      return;
    }
    const cutoff = Math.floor(Date.now() / 1000) - Math.abs(days) * 86_400;
    // A negative step means "older than", which is the other side of the cut.
    set(days > 0 ? { modifiedAfter: cutoff, modifiedBefore: null } : { modifiedAfter: null, modifiedBefore: cutoff });
  };

  const currentAge = filter.modifiedBefore
    ? -365
    : filter.modifiedAfter
      ? Math.round((Math.floor(Date.now() / 1000) - filter.modifiedAfter) / 86_400)
      : null;

  return (
    <div className="filterbar">
      <input
        className="input search"
        type="search"
        placeholder="Search names and paths…"
        value={query}
        onChange={(event) => onQueryChange(event.target.value)}
        aria-label="Search"
      />

      <label className="field">
        Size
        <select
          className="input"
          value={String(filter.minSize ?? "")}
          onChange={(event) =>
            set({ minSize: event.target.value ? Number(event.target.value) : null })
          }
        >
          {SIZE_STEPS.map(([label, value]) => (
            <option key={label} value={value ?? ""}>
              {label}
            </option>
          ))}
        </select>
      </label>

      <label className="field">
        Modified
        <select
          className="input"
          value={String(currentAge ?? "")}
          onChange={(event) => applyAge(event.target.value ? Number(event.target.value) : null)}
        >
          {AGE_STEPS.map(([label, value]) => (
            <option key={label} value={value ?? ""}>
              {label}
            </option>
          ))}
        </select>
      </label>

      <label className="field">
        Extension
        <input
          className="input"
          style={{ width: 92 }}
          placeholder="mp4, zip"
          value={(filter.extensions ?? []).join(", ")}
          onChange={(event) =>
            set({
              extensions: event.target.value
                .split(",")
                .map((part) => part.trim().replace(/^\./, "").toLowerCase())
                .filter(Boolean),
            })
          }
        />
      </label>

      <span className="spacer" />

      <button
        className={`chip${filter.includeHidden ? " on" : ""}`}
        onClick={() => set({ includeHidden: !filter.includeHidden })}
        title="Show files and folders the system hides"
      >
        Hidden
      </button>
      <button
        className={`chip${filter.includeSystem ? " on" : ""}`}
        onClick={() => set({ includeSystem: !filter.includeSystem })}
        title="Show files belonging to macOS itself"
      >
        System
      </button>

      <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
        {CATEGORIES.map((category) => (
          <button
            key={category}
            className={`chip${filter.categories?.includes(category) ? " on" : ""}`}
            onClick={() => toggleCategory(category)}
          >
            {CATEGORY_LABELS[category]}
          </button>
        ))}
      </div>
    </div>
  );
}
