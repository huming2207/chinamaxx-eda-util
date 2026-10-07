# Validation evidence

Validated on Linux on 2026-10-07 with Rust 1.95.0, GPUI 0.2.2,
gpui-component 0.5.1, and KiCad 10.0.6.

## Automated checks

`cargo test --offline --workspace` passed **34 tests**:

- 20 core integration tests: equivalent Eagle/KiCad coordinates, bottom rotation,
  DNP, independent exclusions, grouping/CSV escaping, missing assignments,
  stale projects, validated edits and rollback, non-finite rejection, duplicate
  references, lossless LCSC edits, source parsing, protected outputs, local
  catalogue filtering, supplier error schemas, two real EasyEDA conversions,
  units/drills, missing pin mappings, and unsupported pad rejection.
- 7 picker tests: schematic/project loading, automatic save and resume, failed-save rollback,
  BOM-only export, missing placements, source-change protection, native netlist flags,
  exact requested CSV columns/quantities, and clearing/resuming LCSC assignments.
- 1 importer regression test: paired-board BOM exclusion remains independent of CPL inclusion.
- 3 cache tests: persistence across instances, retry after failure, backend expiry,
  and validation of the 7–60-day bounds.
- 3 CLI integration tests: assignment/export workflow, unknown option protection,
  normalized duplicate API searches make one mock HTTP request across processes,
  and TTL settings persist while invalid values leave the setting unchanged.

The GUI compiled and linked as a native executable. Linux linking used the
installed runtime xkbcommon libraries via temporary local development symlinks;
`scripts/build-linux.sh` makes this reproducible without modifying system files.
No compiler warnings remain in the final build/test runs.

The stable toolchain does not have rustfmt installed. The locally available
older rustfmt was run directly in edition-2018 parsing mode over the source;
the application itself compiles as Rust 2021. Standard `cargo fmt --all` can be
used once the stable rustfmt component is available.

## Native CAD checks

`python3 scripts/validate-native.py` passed against KiCad **10.0.6**.
Evidence/artifacts are in:

`output/native-validation-20261007-130638/validation.json`

The script verifies:

1. CLI read, assignment, board-copy and export for Eagle and KiCad fixtures.
2. Exact BOM rows, DNP omission, and top/bottom placement coordinates/rotations.
3. Refusal to overwrite an existing manufacturing output directory.
4. Native KiCad loading of board copies, including a newly inserted LCSC property.
5. KiCad Gerber and Excellon generation with nonempty output sets.
6. Real upstream C2320 and C427602 EasyEDA fixtures produce symbols that
   `kicad-cli sym export svg` renders, and footprints that `pcbnew.FootprintLoad`
   loads with the expected three/five pads.
7. Eagle libraries have consistent pin-pad connects and mirrored Y coordinates.
   KiCad's independent Eagle library importer loads both generated packages.

KiCad's Python bindings emitted enum/image-handler diagnostic messages; they did
not prevent the checks. No DRC-clean claim is made for the deliberately minimal,
unrouted test boards. Eagle itself is not installed: native Eagle Library Editor,
schematic placement and forward/back annotation have not been tested.

## Environment boundaries and remaining validation

- GPUI startup outside the sandbox completed an eight-second smoke run with the
  copied SWD adaptor project and no logged errors; the process was then stopped
  by `timeout`. Interactive layout, file dialogs and mouse-driven picking have
  not been manually verified.
- One live JLC API search for C25804 succeeded outside the sandbox. Repeating it
  with curl unavailable returned the same result from the persistent cache.
  Other queries, live EasyEDA import, and supplier availability remain unverified.
- Multi-unit conversion code exists, but the two native fixture checks cover
  single-unit parts; broader multi-unit assets need native validation.
- There is no claim of full upstream feature parity. 3D models, Eagle CAM,
  automatic library registration, native schematic edits, binary Eagle formats,
  live editor integration and named variants remain outside this initial release.

## Project picker and supplied CSV examples

`python3 scripts/validate-picker.py PROJECT.kicad_pro BOM-example.csv CPL-example.csv`
was run on a copy of a supplied hierarchical KiCad project. Original hardware files were not changed.
Project-specific validation artifacts were removed after checking the results.

- 90 components loaded through the three-sheet hierarchy; five CAD source files hashed.
- All 35 BOM rows match the supplied the supplied BOM CSV exactly as parsed CSV.
- CPL headers, 90 references, values, packages and layers match the supplied example.
- 14 placements differ geometrically: this app uses footprint origins and does not
  import the other plugin's package-specific rotation/centre corrections. The report
  records the X/Y/rotation deltas needed to reproduce those values. Existing per-part
  correction controls can store these adjustments; they were not silently applied.
- Picking in one CLI process and reopening in another retained the assignment.
- A modified child schematic was detected and resume was rejected without losing picks.
- The saved version 2 JSON passed validation against `docs/project.schema.json`.

## Table layout and themes

The rebuilt GUI was captured on X11 with a supplied 169-component Eagle project. Light and dark rendering, the table headers, natural reference
order, and the separate inspector/export layout were visually inspected at full
and reduced window sizes. Theme switching uses GPUI's built-in light/dark palettes.
The user also confirmed the revised UI looked better. Complete mouse-driven
picking and saving still require manual interaction checks.
