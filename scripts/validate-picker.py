#!/usr/bin/env python3
"""Validate picker resume and CSV compatibility on a COPY of a KiCad project.
Usage: scripts/validate-picker.py PROJECT.kicad_pro BOM-example.csv CPL-example.csv
Requires the built CLI and kicad-cli. Original CAD and CSV files are read only.
"""
import csv
import datetime
import json
from pathlib import Path
import shutil
import subprocess
import sys

project, bom_example, cpl_example = map(lambda s: Path(s).resolve(), sys.argv[1:])
root = Path(__file__).resolve().parents[1]
exe = root / 'target/debug/chinamaxxbom'
out = root / 'output' / ('picker-validation-' + datetime.datetime.now().strftime('%Y%m%d-%H%M%S'))
out.mkdir(parents=True)
for path in project.parent.rglob('*'):
    if path.suffix in ('.kicad_sch', '.kicad_pro', '.kicad_pcb'):
        dst = out / path.relative_to(project.parent)
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dst)

def cli(*args):
    p = subprocess.run([str(exe), *map(str, args)], capture_output=True, text=True)
    assert p.returncode == 0, p.stderr
    return p.stdout

def csv_rows(path):
    with path.open(newline='') as f:
        reader = csv.DictReader(f)
        return reader.fieldnames, list(reader)

copy = out / project.name
opened = json.loads(cli('open', copy))
sidecar = Path(opened['project'])
cli('export', sidecar, '--out', out / 'production')
bom_headers, bom = csv_rows(out / 'production/BOM.csv')
cpl_headers, cpl = csv_rows(out / 'production/CPL.csv')
expected_bom_headers, expected_bom = csv_rows(bom_example)
expected_cpl_headers, expected_cpl = csv_rows(cpl_example)
assert bom_headers == expected_bom_headers
assert cpl_headers == expected_cpl_headers
assert bom == expected_bom, 'BOM content differs'
actual = {r['Designator']: r for r in cpl}
expected = {r['Designator']: r for r in expected_cpl}
assert actual.keys() == expected.keys()
differences = []
for reference in sorted(actual):
    a, b = actual[reference], expected[reference]
    assert all(a[k] == b[k] for k in ('Val', 'Package', 'Layer'))
    dx = float(b['Mid X']) - float(a['Mid X'])
    dy = float(b['Mid Y']) - float(a['Mid Y'])
    rotation = (float(b['Rotation']) - float(a['Rotation']) + 180) % 360 - 180
    if max(abs(dx), abs(dy), abs(rotation)) > 1e-6:
        differences.append(dict(reference=reference, dx=dx, dy=dy, rotation=rotation))

saved = json.loads(sidecar.read_text())
# Separate CLI processes prove persistence, not merely in-memory reuse.
reference = next(r['reference'] for r in saved['bom']['parts'] if not r['exclude_bom'])
cli('assign', sidecar, reference, 'C123456')
resumed = json.loads(cli('open', copy))
assert next(p for p in resumed['bom']['parts'] if p['reference'] == reference)['lcsc'] == 'C123456'
# Restore the saved original selection before leaving the copied project.
sidecar.write_text(json.dumps(saved, indent=2) + '\n')
# Every associated sheet/board hash must guard resume.
child = next(Path(p) for p in saved['source_hashes'] if Path(p).suffix == '.kicad_sch' and Path(p).stem != project.stem)
original = child.read_bytes()
child.write_bytes(original + b'\n')
p = subprocess.run([str(exe), 'open', str(copy)], capture_output=True, text=True)
assert p.returncode != 0 and 'CAD source changed' in p.stderr
child.write_bytes(original)
report = dict(components=len(saved['bom']['parts']), source_files=len(saved['source_hashes']),
              bom_rows=len(bom), cpl_rows=len(cpl), exact_bom_match=True,
              cpl_headers_and_references_match=True, resumed_pick=True, child_sheet_hash_checked=True,
              cpl_corrections_needed_to_match_example=differences)
(out / 'validation.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
print('Artifacts:', out)
