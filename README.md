# ChinamaxxBOM

A Rust application for **Eagle XML projects/schematics/boards and KiCad 10+**, with a native **GPUI desktop UI** and a CLI sharing the same implementation.

This is an initial working implementation of the part-library and assembly workflows researched in [JLCImport](https://github.com/jvanderberg/kicad_jlcimport) and [KiCad JLCPCB Tools](https://github.com/Bouni/kicad-jlcpcb-tools). It is not yet feature-equivalent to either mature plugin. See [implementation status](docs/implementation.md) for supported geometry and remaining work.

## Build and run

```sh
cargo build --release --workspace
./target/release/chinamaxxbom-gui
./target/release/chinamaxxbom --help
```

On Linux, GPUI requires a working Wayland/X11 desktop, Vulkan graphics support, and the xkbcommon development libraries. On Debian/Ubuntu, the usual build prerequisites include `build-essential`, `pkg-config`, `libxkbcommon-dev`, and `libxkbcommon-x11-dev`. Online supplier access uses `curl` with certificate verification and request timeouts. KiCad 10+ is needed to read KiCad schematics (native netlist export), and for native validation and Gerber/drill generation. Eagle XML and direct board reading do not require a CAD executable. Python is only used by the optional native validation script.

For the CLI alone:

```sh
cargo build --release -p chinamaxxbom-cli
```

The workspace has three crates: `chinamaxxbom-core`, `chinamaxxbom-cli` (binary `chinamaxxbom`), and `chinamaxxbom-gui`. `Cargo.lock` is included. Dependencies are managed by Cargo and fetched from crates.io. The development binaries are also available in `target/debug/` after validation.

On machines with xkbcommon runtime libraries but no development symlinks, [scripts/build-linux.sh](scripts/build-linux.sh) supplies local linker symlinks without changing the system. Installing the development packages is the normal build setup.

## Desktop workflow

1. Start `chinamaxxbom-gui [path]`. Open a `.kicad_pro`, Eagle `.epf`, `.kicad_sch`, Eagle XML `.sch`, board, project directory, or saved JSON. The BOM loads automatically; KiCad includes hierarchical sheets and merges multi-unit symbols through its native netlist exporter. Eagle uses the embedded device/package definitions, not schematic drawing positions.
2. Select a row in **Bill of materials**, then use **Find JLCPCB part** or open **JLCPCB catalogue**. Searches use JLC's API and the shared cache. Select a catalogue row and click **Use selected part**: the choice saves immediately to `<input>.chinamaxxbom.json` without another confirmation. Manual LCSC/DNP/placement adjustments use **Save changes**.
3. Reopen the same input or its JSON to resume. Every completed selection is atomically saved as indented JSON. A crash during saving leaves the previous complete file or the new complete file. See [the JSON schema](docs/project.schema.json) and [saved project format](docs/project-format.md).
4. Choose a new output folder and export. A matching same-basename `.kicad_pcb` or `.brd` supplies CPL placement. Without a board, export produces a BOM only. If a paired board is missing required schematic references, export stops and identifies them. KiCad boards can optionally generate Gerbers and Excellon files.
5. **Library import** accepts an LCSC ID or an offline bundle, and writes KiCad/Eagle libraries into a new folder.

The BOM and catalogue use resizable tables with fixed column headers and horizontal/vertical scrolling. The component inspector scrolls independently; export controls remain below it. References use natural ordering (`C2` before `C10`). The toolbar **Dark mode / Light mode** button changes the whole interface; each launch starts with the desktop theme.

CAD files are not changed by assignment or export. The CLI `write-board` command can produce a separate board copy containing the LCSC field edits. DNP and placement corrections remain in the ChinamaxxBOM sidecar. Eagle board copies must not replace the board of a paired schematic without handling Eagle's forward/back annotation yourself.

## CLI examples

All paths are examples; output directories must not already exist.

```sh
# Open a project/schematic and automatically create or resume its saved picks.
chinamaxxbom open design.kicad_pro
chinamaxxbom open design.sch

# Examine board data without changing it.
chinamaxxbom inspect design.brd
chinamaxxbom inspect design.kicad_pcb

# Create assignments tied to the saved board's SHA-256.
chinamaxxbom init design.brd --project assembly.json
chinamaxxbom assign assembly.json R1 C25804
chinamaxxbom dnp assembly.json C3 true
chinamaxxbom correct assembly.json U1 --dx 0.25 --dy -0.5 --rotation 90
chinamaxxbom export assembly.json --out production/revision-1
chinamaxxbom write-board assembly.json --out design-assigned.brd

# Native Gerber/Excellon generation, for a KiCad project.
chinamaxxbom export kicad-assembly.json --out production/revision-2 --gerbers

# Online and offline search; stdout is JSON.
chinamaxxbom search '100nF 0402' --basic --in-stock
chinamaxxbom search '10k 0603' --catalog tests/fixtures/catalog.json

# Preserve fetched raw data for repeatable offline imports.
chinamaxxbom fetch C2320 --out C2320.json
chinamaxxbom import --bundle C2320.json --out libraries/C2320 --target both
# Or fetch and import in one command.
chinamaxxbom import --lcsc C2320 --out libraries/C2320-online --target both
```

For a completely offline demonstration with the included upstream fixtures:

```sh
mkdir -p output
./target/debug/chinamaxxbom bundle C2320 \
  --symbol tests/fixtures/C2320_symbol.json \
  --footprint tests/fixtures/C2320_footprint.json \
  --out output/C2320.json
./target/debug/chinamaxxbom import --bundle output/C2320.json \
  --out output/C2320-library --target both
```

`bundle` is a convenience for one symbol unit. A multi-unit bundle uses the JSON schema `{ "lcsc": "C123", "symbols": [component, ...], "footprint": component }`. Components may be raw EasyEDA results or `{"result": ...}` response envelopes. An offline search catalogue is either an array of normalized parts or the JSON output of `search`; example stock/prices are synthetic and explicitly labelled.

## Direct API search and configurable cache

The component picker calls **JLCPCB's API directly**. It never downloads a SQLite parts database. The GUI and CLI share a persistent `cached`/`redb` response cache with a **7-day default TTL, configurable from 7 to 60 days**. Cache keys include normalized search text, page number, basic/extended filter and stock filter. EasyEDA component lookup/geometry responses also use the same TTL.

Repeated searches use disk cache across app restarts. Concurrent requests for the same missing entry are coalesced with a process-shared file lock. Uncached requests are spaced at least three seconds apart across the GUI and CLI; typing does not issue requests, and errors are not automatically retried. Only validated successful responses are cached; expired responses are refreshed, and fetch failures are surfaced instead of silently returning stale stock.

Set the TTL in the GUI JLCPCB catalogue panel, or run `chinamaxxbom cache-days 30`. Run `chinamaxxbom cache-days` to read it. The setting is shared by the GUI and CLI and applies on subsequent requests, including existing entries; cache hits do not extend expiry. Storage and expiry are handled by the `cached` crate.

The cache uses the standard user cache directory on Linux, respecting `XDG_CACHE_HOME`. Set `CHINAMAXXBOM_CACHE_DIR` to choose another directory. Search JSON includes `cache.hit`, `cache.fetched_at` and `cache.expires_at` (Unix seconds); the GUI labels cached results. Cached stock/prices can be as old as the configured TTL. The optional offline JSON catalogue is a user-supplied file, not a downloaded supplier database.

## Generated files and coordinates

Assembly exports contain `BOM.csv`, `CPL.csv` (when a board is available), `report.json`, and the assignment `project.json`. The CSV format matches the supplied CSV examples:

- BOM: `Comment,Designator,Footprint,LCSC,Quantity`
- CPL: `Designator,Val,Package,Mid X,Mid Y,Rotation,Layer`

BOM rows group identical values, packages, and LCSC IDs. Footprint library prefixes are removed, quantities are explicit, layers are lowercase, coordinates are millimetres, and CSV uses CRLF with quoting where needed. Rotations are equivalent signed angles in (-180, 180]. With `--gerbers`, a `gerbers/` directory contains KiCad's native fabrication files. The entire export is staged before publication. Existing output directories and library destinations are refused. Missing LCSC assignments are errors unless `--allow-missing` is explicitly selected. DNP parts are omitted from both BOM and CPL; native KiCad BOM/position exclusions are respected independently.

The shared placement coordinates are millimetres with X right, Y up. KiCad coordinates are relative to the auxiliary origin and invert Y. Eagle uses its drawing origin. Bottom rotation follows the `(180 - board rotation) mod 360` convention researched in JLCPCB Tools; an explicit correction is then added. This release uses the **footprint/element origin**, not the upstream plugin's pad bounding-box centre. `dx`/`dy` corrections are in final assembly/world axes, not footprint-local axes. Always review placement centres, bottom rotations, and pin 1 in the manufacturer's preview.

Projects record hashes of the input, hierarchical schematic files, and associated board. If a source changes, the app stops rather than applying stale assignments to a new layout. Currently you must create a fresh project and review/reapply assignments; an assisted rebase workflow is not implemented.

Library output contains `<C-ID>.kicad_sym`, `<C-ID>.pretty/<C-ID>.kicad_mod`, and/or `<C-ID>.lbr`, plus `report.json`. Generated KiCad symbols reference `<C-ID>:<C-ID>`. Add the `.kicad_sym` and `.pretty` directories under that nickname in KiCad's library managers. The generated `sym-lib-table`/`fp-lib-table` work if the library output directory is also the KiCad project directory; do not overwrite an existing project's tables with them. Eagle libraries contain packages, symbols, devicesets, gates, explicit pin-to-pad connects, and an LCSC technology attribute.

## Validation

```sh
cargo test --workspace
python3 scripts/validate-native.py
```

The native script requires a built debug CLI, KiCad 10+, and system Python with `pcbnew`. It leaves inspectable exports under `output/native-validation-<timestamp>/` and a `validation.json` report. See [validation evidence](docs/validation.md) for the tested environment and boundaries.

## Local Git repository

The local repository uses a normal `.git` directory and the **master** branch. Use ordinary `git status`, `git log`, and `git diff`. No remote is configured and nothing is pushed.


## Upstream references and licensing

The reference implementations are [kicad_jlcimport](https://github.com/jvanderberg/kicad_jlcimport) and [kicad-jlcpcb-tools](https://github.com/bouni/kicad-jlcpcb-tools). Pinned revisions and research notes are in [docs/implementation.md](docs/implementation.md).

This implementation is MIT licensed. Imported supplier assets remain subject to their providers' terms.
