#!/usr/bin/env python3
"""End-to-end CLI + KiCad 10 native artifact validation. Requires system pcbnew.
Produces inspectable artifacts below output/native-validation-<timestamp>.
"""
import csv
import datetime
import json
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
EXE = ROOT / 'target/debug/chinamaxxbom'
OUT = ROOT / 'output' / ('native-validation-' + datetime.datetime.now().strftime('%Y%m%d-%H%M%S'))
OUT.mkdir(parents=True)

def run(*args):
    p = subprocess.run([str(x) for x in args], cwd=ROOT, capture_output=True, text=True)
    if p.returncode:
        raise RuntimeError(f'{args}\n{p.stdout}\n{p.stderr}')
    return p.stdout

def cli(*args):
    return run(EXE, *args)

version = run('kicad-cli', 'version').strip()
assert int(version.split('.')[0]) >= 10, version
for ext in ('brd', 'kicad_pcb'):
    project = OUT / f'{ext}.json'
    cli('init', ROOT / f'tests/fixtures/assembly.{ext}', '--project', project)
    cli('assign', project, 'R1', 'C25804')
    copy = OUT / f'assigned.{ext}'
    cli('write-board', project, '--out', copy)
    data = json.loads(cli('inspect', copy))
    assert len(data['parts']) == 3
    cli('export', project, '--out', OUT / ext, *(['--gerbers'] if ext == 'kicad_pcb' else []))
    with (OUT / ext / 'BOM.csv').open(newline='') as f:
        rows = list(csv.DictReader(f))
        assert rows == [{'Comment': '10k', 'Designator': 'R1,R2', 'Footprint': 'R_0603', 'LCSC': 'C25804', 'Quantity': '2'}], rows
    with (OUT / ext / 'CPL.csv').open(newline='') as f:
        rows = list(csv.DictReader(f))
        assert [(r['Designator'], r['Mid X'], r['Mid Y'], r['Layer'], r['Rotation']) for r in rows] == [
            ('R1', '2.0', '3.0', 'top', '90.0'),
            ('R2', '6.0', '8.0', 'bottom', '150.0')]
    rejected = subprocess.run([str(EXE), 'export', str(project), '--out', str(OUT / ext)], capture_output=True)
    assert rejected.returncode != 0

import pcbnew
native = pcbnew.LoadBoard(str(OUT / 'assigned.kicad_pcb'))
assert len(list(native.GetFootprints())) == 3
# Also exercise insertion of a previously absent LCSC property, not just replacement.
cli('assign', OUT / 'kicad_pcb.json', 'C1', 'C14663')
cli('write-board', OUT / 'kicad_pcb.json', '--out', OUT / 'inserted.kicad_pcb')
native_inserted = pcbnew.LoadBoard(str(OUT / 'inserted.kicad_pcb'))
assert len(list(native_inserted.GetFootprints())) == 3
assert list((OUT / 'kicad_pcb/gerbers').glob('*.gbr')), 'Missing Gerbers'
assert list((OUT / 'kicad_pcb/gerbers').glob('*.drl')), 'Missing drills'

for part, count in [('C2320', 3), ('C427602', 5)]:
    bundle = OUT / f'{part}.json'
    cli('bundle', part, '--symbol', ROOT / f'tests/fixtures/{part}_symbol.json', '--footprint', ROOT / f'tests/fixtures/{part}_footprint.json', '--out', bundle)
    lib = OUT / part
    cli('import', '--bundle', bundle, '--out', lib, '--target', 'both')
    run('kicad-cli', 'sym', 'export', 'svg', '--output', lib / 'svg', lib / f'{part}.kicad_sym')
    assert list((lib / 'svg').glob('*.svg')), 'KiCad produced no symbol SVG'
    fp = pcbnew.FootprintLoad(str(lib / f'{part}.pretty'), part)
    assert fp is not None
    assert len(list(fp.Pads())) == count
    pads = {p.GetNumber(): (pcbnew.ToMM(p.GetPosition().x), pcbnew.ToMM(p.GetPosition().y)) for p in fp.Pads()}
    doc = ET.parse(lib / f'{part}.lbr')
    package = doc.find('.//package')
    epads = list(package.findall('pad')) + list(package.findall('smd'))
    assert len(epads) == count
    for pad in epads:
        kp = pads[pad.attrib['name']]
        assert abs(kp[0] - float(pad.attrib['x'])) < 0.000002
        assert abs(kp[1] + float(pad.attrib['y'])) < 0.000002
    pins = {p.attrib['name'] for p in doc.findall('.//symbol/pin')}
    connects = doc.findall('.//connect')
    assert len(connects) == count
    assert all(c.attrib['pin'] in pins and c.attrib['pad'] in pads for c in connects)
    # KiCad's Eagle plugin independently reads generated Eagle package geometry.
    eagle_fp = pcbnew.FootprintLoad(str(lib / f'{part}.lbr'), part)
    assert eagle_fp is not None
    assert len(list(eagle_fp.Pads())) == count

summary = {'kicad': version, 'status': 'passed', 'checks': [
    'Eagle and KiCad CLI inspect/assign/write/export',
    'BOM grouping, DNP and top/bottom CPL coordinates',
    'Refusal to overwrite manufacturing outputs',
    'KiCad native board loading and new LCSC property insertion',
    'KiCad Gerber and Excellon generation',
    'Two real EasyEDA fixtures: native symbol SVG and footprint loading',
    'Eagle pin-pad connectivity, mirrored Y and native KiCad Eagle package import',
]}
(OUT / 'validation.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
print(f'Artifacts: {OUT}')
