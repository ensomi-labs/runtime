#!/usr/bin/env python3
"""Compare a new worker with saved baseline .osu bytes, generating fresh candidates.
Run after the baseline sweep. This does not use the candidate cache for the new build.
"""
import argparse, concurrent.futures, hashlib, json
from pathlib import Path
from run import Worker, dump, measure, load_calculator, GRID, SEEDS, PUBLISHED


def run_song(args,song,calc):
    reference=json.loads((args.private/'sweeps/baseline'/(song['id']+'.json')).read_text())
    w=Worker(args); results=[]
    try:
        for row in reference:
            r=row['star_request_input']
            if not (2.125<=r<=5.875 or r in [1,9]):continue
            opts_list=[{}]
            if song['published'] and r in [2.125,3.5,5.875]:opts_list += [{'rows':'overall','rows_min':0.,'rows_max':6.},{'rows':'end_segment','rows_min':0.,'rows_max':6.}]
            for opts in opts_list:
                key=hashlib.sha256(json.dumps([r,row['seed'],opts],sort_keys=True).encode()).hexdigest()[:20]
                path=args.private/'parity-charts'/song['id']/key;path.parent.mkdir(parents=True,exist_ok=True)
                value=w.call(dict(op='generate',prepared=song['prepared'],star=r,seed=row['seed'],options=opts,out=str(path)))
                measured=measure(path,value['song_ms'],calc)
                same=measured['osu_sha256']==row['osu_sha256']
                results.append(dict(request=r,seed=row['seed'],options=opts,same_osu=same,same_star=measured['star']==row['star']))
                if not same:raise RuntimeError(f'chart differs: {song["id"]} {r} {opts}')
    finally:w.close()
    dump(args.private/'parity-charts'/song['id']/'checks.json',results)
    return results


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--model-repo',type=Path,required=True)
    p.add_argument('--private',type=Path,required=True)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--onnx',type=Path)
    p.add_argument('--assets',type=Path,default=Path('assets/resampler'))
    p.add_argument('--jobs',type=int,default=2)
    args=p.parse_args();args.private=args.private.resolve();args.model_repo=args.model_repo.resolve();args.binary=args.binary.resolve();args.assets=args.assets.resolve()
    args.onnx=args.onnx or args.model_repo/'artifacts/release-r2/bulk/hf/ensomi-r2/onnx'
    calc=load_calculator(args);songs=json.loads((args.private/'manifest.json').read_text())['songs']
    results=[]
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for r in pool.map(lambda s:run_song(args,s,calc),songs):results+=r
    summary=dict(candidates=len(results),same_osu=sum(r['same_osu'] for r in results),same_star=sum(r['same_star'] for r in results),default_candidates=sum(not r['options'] for r in results),explicit_rows_options=sum(bool(r['options']) for r in results))
    dump(args.private/'default-parity.json',summary);print(json.dumps(summary))

if __name__=='__main__':main()
