# Investigation and implementation status

## Reference sources

| Source | Revision | What was investigated |
| --- | --- | --- |
| `jvanderberg/kicad_jlcimport` | `d98bf321dbae67dbf26cbc8e482c206b79f02f8f` | EasyEDA UUID lookup, supplier search payload/results, 10-mil coordinate units, drill radius, display pin numbers, multi-unit symbols, KiCad format versions |
| `Bouni/kicad-jlcpcb-tools` | `36852796f92b7e97d7856ae84e93e8308e0d80a3` | BOM grouping/exclusions, assembly origins, bottom rotation, offset conventions, catalogue APIs, native manufacturing exports |

The two JLC repositories were consulted at the revisions listed above. Both are MIT licensed.

Also consulted: [KiCad board format](https://dev-docs.kicad.org/en/file-formats/sexpr-pcb/), installed KiCad 10.0.6 CLI help/native parsers, and locally cached GPUI/kurbo crate APIs. No runtime plugin/Python dependency is used.

## Implemented

| Capability | Eagle | KiCad |
| --- | --- | --- |
| Read project/schematic BOM | `.epf`, XML `.sch`; embedded devices and technologies | `.kicad_pro`, `.kicad_sch`; native hierarchical netlist via KiCad 10+ |
| Read board components, placement, LCSC attributes | Eagle 6+ XML `.brd` | Modern `.kicad_pcb`; tested on 10.0.6 |
| Native GPUI UI and CLI | Shared core | Shared core |
| Online JLC catalogue search and offline catalogue search | Shared | Shared |
| Assign LCSC IDs, DNP, X/Y/rotation corrections | Sidecar project | Sidecar project |
| Write separate board copy with LCSC fields | Yes, preserves unrelated bytes | Yes, preserves unrelated bytes |
| BOM/CPL generation | Yes | Yes |
| Gerber and Excellon | Not yet; use Eagle CAM | `kicad-cli` 10+ |
| EasyEDA supplier symbol + footprint import | `.lbr` with connects | `.kicad_sym` + `.pretty` |
| Multi-unit symbols | Gates/symbols | Numbered symbol units |
| Stock/basic filtering, price/datasheet display | Shared | Shared |
| 3D assets | Not implemented | Not implemented |
| Live editor integration/selection | Not implemented | Not implemented |
| Native schematic write-back or variants | Not implemented | Not implemented |

## Library conversion scope

The canonical geometry is mm, positive Y up. EasyEDA Standard shapes use exactly 0.254 mm per coordinate unit. Electrical pads support RECT, ELLIPSE and OVAL; top/bottom SMD and round-drilled through-hole pads are supported. Eagle through-hole pads are restricted to equal width/height. Unsupported custom polygon pads, slots, missing pin-pad mappings, unknown layers and unsupported substantive primitives are errors. Duplicate physical pad numbers are accepted by the KiCad writer, but rejected by the Eagle writer until proper split-pad mapping is implemented.

Footprint tracks, circles, holes, rectangles, simple filled regions and SVG arcs are supported. SVG curves are flattened using kurbo (nominal 0.01 mm curve tolerance); circles use bounded polygonal approximation. Compound filled paths with holes are rejected. Cosmetic EasyEDA helper layers 99/100/101 are omitted. Decorative footprint/symbol text and 3D metadata produce import warnings. Symbol rounded rectangles use square corners and warn. Decorative symbol fill/line styling is simplified. Generated reference/value labels are provided.

Pin coordinates/direction are derived from the actual connection path, with the displayed pad number preferred over the SPICE index. Pin paths must be straight. Eagle pin lengths must match its point/short/middle/long values and its angles must be orthogonal. Units must contain pins, and each symbol pin must map to an existing electrical pad. Unmapped extra pads produce a warning for review (for example, mechanical mounting pads).

## Architectural decisions

- `chinamaxxbom-core` owns parsers, assignments, supplier access, conversion and exports.
- `chinamaxxbom-cli` has no GUI dependencies and emits JSON for inspection/search/reporting.
- `chinamaxxbom-gui` uses GPUI and gpui-component; network and file jobs run on the background executor.
- File-based adapters avoid depending on KiCad's evolving editor APIs. Eagle is parsed as XML; KiCad is parsed as bounded S-expressions retaining source spans for surgical LCSC edits.
- Version 2 sidecars store a BOM snapshot, per-reference picks, and SHA-256 hashes of CAD inputs. Supplier picks save immediately and reopening resumes the sidecar. Exports and imported libraries are staged and refuse existing destinations.
- Search directly calls JLCPCB, with no SQLite database. The `cached` crate with its persistent `redb` backend provides a default seven-day TTL configurable from 7–60 days, query/filter/page keys, process-shared miss coalescing and three-second request spacing. Search only runs on explicit submit; cached stock age is visible. EasyEDA responses share the cache policy.
- `curl` provides HTTPS, platform certificate/proxy support, bounded request timeouts and HTTP errors. No disabled certificate checks or hard-coded IP fallbacks.
- Per-part corrections are explicit. Upstream correction databases are not silently applied, since their footprint/library keys may not identify Eagle packages correctly.

## Remaining work

This is a working first version, not full parity with both upstream plugins. Highest-priority follow-ups:

1. Interactive Linux UI validation on an accessible desktop and representative user boards.
2. A live online supplier smoke test where network access is available; catalogue schemas have offline tests, but the supplier API can change.
3. Broader real-board/Eagle validation, pad-centre placement policy, and schematic/board field synchronization.
4. More EasyEDA primitives, accurate decorative style/text, custom copper pads and slots; 3D download, transform and library registration.
5. Native Eagle CAM, fabrication ZIP packaging, DRC integration, named variants and explicit correction-database import.
6. Efficient virtualized tables for very large boards/catalogues, board/footprint previews, keyboard workflows and application packaging.
7. Assisted project rebase after board edits, portable relative project paths, and direct native board-copy export in the GUI.

Binary Eagle files (pre-6), Fusion cloud documents and `.f3d` files are not accepted. Export Eagle XML from the originating tool first. Future KiCad versions are supported only to the extent they retain the parsed board fields; only 10.0.6 has been natively validated here.
