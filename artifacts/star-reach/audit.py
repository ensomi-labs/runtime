#!/usr/bin/env python3
"""Check cached measurements still name the exact chart and receipt that produced them."""
import argparse,hashlib,json
from pathlib import Path
from run import dump,digest
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--private',type=Path,required=True);a=p.parse_args()
checks=0;errors=[]
for path in a.private.glob('candidates/*/*.metrics.json'):
    r=json.loads(path.read_text());prefix=Path(str(path)[:-len('.metrics.json')])
    identity={k:r[k] for k in ['star_request_input','seed','options']}
    key=hashlib.sha256(json.dumps(identity,sort_keys=True).encode()).hexdigest()[:20]
    if prefix.name!=key:errors.append(str(path)+': identity')
    if 'error' in r:continue
    receipt=json.loads(Path(str(prefix)+'.receipt.json').read_text())
    if digest(str(prefix)+'.osu')!=r['osu_sha256']:errors.append(str(path)+': chart bytes')
    if receipt['star']!=r['star_request_input'] or receipt['seed']!=r['seed']:errors.append(str(path)+': request')
    for key,value in r['options'].items():
        if receipt.get('experiment',{}).get(key)!=value:errors.append(str(path)+': option '+key)
    d=receipt['controls']['difficulty']
    if d['m_final']!=r['multiplier'] or d['achieved_r']!=r['r'] or d['q']!=r['q']:errors.append(str(path)+': receipt values')
    if receipt['rows']['K']!=r['head_rows'] or receipt['export']['objects']!=r['notes']:errors.append(str(path)+': row/note count')
    checks+=1
summary=dict(successful_candidates_checked=checks,mismatches=len(errors),passed=not errors)
dump(a.private/'cache-audit.json',dict(summary,details=errors));print(json.dumps(summary))
if errors:raise SystemExit(1)
