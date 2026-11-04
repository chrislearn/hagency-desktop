#!/usr/bin/env python3
"""Check catalog parity, placeholders, and explicit Rust UI translation keys."""
import json
import re
from itertools import chain
from pathlib import Path


def main():
    root=Path(__file__).resolve().parents[2]
    en=json.loads((root/'resources/i18n/en.json').read_text(encoding='utf-8'))
    zh=json.loads((root/'resources/i18n/zh-CN.json').read_text(encoding='utf-8'))
    assert en.keys()==zh.keys(), 'English/Chinese catalog keys differ'
    def fields(text):return sorted(set(re.findall(r'\{([^{}]+)\}',text)))
    for key,value in en.items():
        assert key==value, 'English source keys must retain their English fallback'
        assert zh[key].strip(), 'Empty translation: '+key
        assert fields(key)==fields(zh[key]), 'Placeholder mismatch: '+key
    missing=[]
    sites=0
    for path in chain((root/'src').rglob('*.rs'), (root/'apps').rglob('*.rs')):
        for match in re.finditer(r'i18n::(?:tr|format)\(\s*("(?:\\.|[^"\\])*")',path.read_text(encoding='utf-8')):
            sites+=1
            key=json.loads(match[1])
            if key not in en:missing.append((str(path.relative_to(root)),key))
    # Mini-app consent translates these contract descriptions dynamically;
    # scanning only literal i18n::tr calls misses newly added permissions.
    contract=root/'vendor/octosense-app-contract/src/hagency.rs'
    descriptions=contract.read_text(encoding='utf-8').split('pub fn words(',1)[1]
    permissions=0
    for match in re.finditer(r'"[^"\\]+"\s*=>\s*("(?:\\.|[^"\\])*")', descriptions):
        permissions+=1
        key=json.loads(match[1])
        if key not in en:missing.append((str(contract.relative_to(root)),key))
    assert not missing, 'Missing UI translations: '+repr(missing)
    print(json.dumps({'catalog_entries':len(en),'translated_call_sites':sites,'management_permissions':permissions,'missing':0},ensure_ascii=False))


if __name__=='__main__':main()
