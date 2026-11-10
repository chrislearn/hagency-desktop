#!/usr/bin/env python3
"""Require Desktop and owner-dashboard to use the in-tree Hagency libraries."""
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
LIBRARY = Path('crates/hagency-client')
MANIFESTS = ['Cargo.toml', 'crates/owner-dashboard/Cargo.toml']


def check(root=ROOT):
    checked = []
    for name in MANIFESTS:
        manifest = root / name
        for package, body in re.findall(r'^(hagency(?:-core|-metering)?)\s*=\s*\{([^\n]+)\}', manifest.read_text(), re.M):
            path = re.search(r'\bpath\s*=\s*"([^"]+)"', body)
            expected = root / LIBRARY / 'native' / package
            if (not path or re.search(r'\b(?:git|rev|branch|tag)\s*=', body)
                    or (manifest.parent / path[1]).resolve() != expected.resolve()
                    or expected.is_symlink()
                    or not (expected / 'Cargo.toml').is_file()):
                raise SystemExit(f'{name}: {package} must use the in-tree library at {expected.relative_to(root)}')
            checked.append(f'{name}:{package}')
    if len(checked) != 4:
        raise SystemExit('Desktop and owner-dashboard must share the in-tree libraries, including test dependencies')
    native_names = set()
    for manifest in (root / LIBRARY / 'native').glob('hagency*/Cargo.toml'):
        native_names.add(re.search(r'^name\s*=\s*"([^"]+)"', manifest.read_text(), re.M)[1])
    locks = ['Cargo.lock', 'crates/owner-dashboard/Cargo.lock', str(LIBRARY / 'Cargo.lock')]
    for name in locks:
        packages = (root / name).read_text().split('[[package]]')[1:]
        native = [p for p in packages if re.search(r'^name = "([^"]+)"', p, re.M)[1] in native_names]
        if not native or any(re.search(r'^source\s*=', p, re.M) for p in native):
            raise SystemExit(f'{name}: Hagency libraries must resolve locally, with no Git or registry copy')
    # Guard the removed UI/build surface against accidental reintroduction.
    for name in ['mockup', 'native/hagency/src/main.rs', 'native/hagency/build.rs', 'native/hagency/src/console/assets.rs']:
        if (root / LIBRARY / name).exists():
            raise SystemExit(f'{LIBRARY / name}: retired standalone UI must not ship in Desktop')
    return {'clientSource': str(LIBRARY), 'checked': checked, 'lockfiles': len(locks)}


if __name__ == '__main__':
    print(json.dumps(check()))
