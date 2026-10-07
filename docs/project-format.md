# Saved picker projects

The app writes `<input-path>.chinamaxxbom.json` automatically. Opening the same
input, or opening this JSON directly, resumes the saved BOM and picks. The formal
version 2 schema is [project.schema.json](project.schema.json).

- `version`: 2. Version 1 board-only sidecars remain readable.
- `input`: the project, schematic or board selected by the user; absolute path.
- `placement_board`: the matching board, omitted when none was found.
- `board` and `source_sha256`: primary source path and SHA-256 (the placement board
  when available, otherwise the selected input). These retain version 1 compatibility.
- `source_hashes`: paths and hashes for the CAD files used, including hierarchical sheets.
- `bom`: original component snapshot, format and import warnings. Each part has
  reference, value, footprint, initial LCSC number, DNP/exclusion flags, and placement.
  `placed: false` means its coordinates are placeholders and must never enter a CPL.
- `edits`: map keyed by reference. Each selection sets `lcsc`; optional `dnp`, `dx`,
  `dy`, and `rotation` retain manual overrides. Offsets use mm; rotations use degrees.

For example, an entry in `edits` can be:

```json
{
  "R1": {
    "lcsc": "C25804",
    "dnp": null,
    "dx": 0.0,
    "dy": 0.0,
    "rotation": 0.0
  }
}
```

Each pick writes indented JSON to a temporary file beside the sidecar, syncs it,
and atomically replaces the sidecar. The UI reports success only after saving.
An interrupted save preserves a complete old or new JSON file. Failed saves are
reported and do not update the in-memory project. API search results have their
own shared cache; the project only needs the selected LCSC identifier to resume.

Source changes stop loading/export rather than silently reuse stale placement.
The saved JSON is kept intact. Create a fresh project and review/reapply picks
when CAD files change. Opening a project and opening its schematic use separate
sidecars; reopen the same input or its JSON to resume. Concurrent editing of the
same sidecar in multiple app instances is not supported.

A schematic uses a same-basename board if present. Project directory discovery
requires an unambiguous project; otherwise select the specific file. Eagle `.epf`
uses its same-basename schematic or the sole `.sch` in the directory. KiCad sheet
paths containing unresolved `${...}` variables are rejected explicitly. Only the
default assembly variant is loaded.

An empty `lcsc` string explicitly clears an assignment; `null` retains the CAD
source value. Clearing the inspector field and saving stores the empty string.
