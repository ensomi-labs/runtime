#!/usr/bin/env python3
"""Offline experiment. Selection alone needs numpy/pyarrow; measurement/search use stdlib.
All per-song output goes to --private. Run --help for paths and subcommands.
"""
from __future__ import annotations
import argparse, bisect, collections, concurrent.futures, hashlib, importlib.util
import json, math, os, statistics, subprocess, sys, time
from pathlib import Path

SEEDS = [0, 1000]
TARGETS = [1 + i / 2 for i in range(17)]
GRID = sorted(set(TARGETS + [1.875, 2.0625, 2.125, 2.1875, 5.75, 5.8125, 5.875, 5.9375, 6.125]))
ARM_GRID = [1, 1.5, 2, 2.125, 5.875, 6.5, 7, 8, 9]
PUBLISHED = ['adventure', 'catch-the-moment', 'hesitation-snow', 'saigetsu']
PIN = '885bb74c19bd68d1c77087d3ec8746244196daa5'


def dump(path, value):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(str(path) + '.tmp')
    temporary.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')
    temporary.replace(path)


def digest(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for b in iter(lambda: f.read(1024 * 1024), b''): h.update(b)
    return h.hexdigest()


def load_calculator(args):
    path = args.private / 'difficulty_r2_1_0_1.py'
    source = subprocess.check_output(['git', '-C', str(args.model_repo), 'show', PIN + ':src/ensomi_model/osu_core/difficulty.py'])
    if path.exists(): assert path.read_bytes() == source
    else: path.write_bytes(source)
    spec = importlib.util.spec_from_file_location('pinned_difficulty', path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def select(args):
    import pyarrow.parquet as pq
    import numpy as np
    dest = args.private / 'manifest.json'
    if dest.exists(): raise RuntimeError('Manifest already frozen; use a new private directory for a new sample')
    corpus = args.model_repo / 'data/r2-corpus.parquet'
    rows = pq.read_table(corpus).to_pylist()
    eligible = []
    published_hashes = set()
    songs = []
    for name in PUBLISHED:
        root = args.model_repo / 'artifacts/runtime-ref/bulk' / name
        index = json.loads((root / 'index.json').read_text())
        entries = {x['name']: x for x in index['arrays']}
        def array(key):
            x = entries[key]
            return np.memmap(root / x['file'], mode='r', dtype=x['dtype'], offset=x['byte_offset'], shape=tuple(x['shape']))
        pcm = array('audio/pcm')
        mono = np.asarray(pcm.mean(axis=1) if pcm.ndim == 2 else pcm, dtype='<f4')
        pcm_path = args.private / 'pcm' / (name + '.f32')
        pcm_path.parent.mkdir(parents=True, exist_ok=True)
        mono.tofile(pcm_path)
        summary = json.loads((args.model_repo / 'artifacts/runtime-ref' / name / 'summary.json').read_text())
        audio = args.model_repo / summary['audio']
        if audio.is_file(): published_hashes.add(digest(audio))
        songs.append(dict(id=name, published=True, pcm=str(pcm_path), sample_rate=int(array('audio/sample_rate')), recording=str(root), seconds=len(mono)/int(array('audio/sample_rate'))))
    seen = set()
    for r in sorted(rows, key=lambda r: r['path']):
        span = (r['last_ms'] - r['first_ms']) / 1000
        bpm = r['dominant_bpm']
        density = (r['n_taps'] + r['n_lns']) / span if span > 0 else 0
        audio = args.model_repo / r['set_dir'] / r['audio_filename']
        sha = r['audio_sha256']
        if r['eval_split'] != 'heldout' or not r['audio_present'] or not audio.is_file() or not (45 <= span <= 300) or not all(math.isfinite(x) and x > 0 for x in [bpm, density]) or sha in seen or sha in published_hashes: continue
        seen.add(sha)
        eligible.append(dict(row=r, audio=str(audio), bpm=bpm, span=span, density=density))
    # Average percentile ranks of ties, independently in each coordinate.
    def ranks(key):
        values = sorted(x[key] for x in eligible)
        return [(bisect.bisect_left(values, x[key]) + bisect.bisect_right(values, x[key]) - 1) / (2 * (len(values)-1)) for x in eligible]
    coordinates = list(zip(*(ranks(k) for k in ['bpm', 'span', 'density'])))
    chosen = [min(range(len(eligible)), key=lambda i: (eligible[i]['bpm'], eligible[i]['row']['path']))]
    while len(chosen) < 8:
        distances = {i: min(sum((a-b)**2 for a,b in zip(coordinates[i], coordinates[j])) for j in chosen) for i in range(len(eligible)) if i not in chosen}
        chosen.append(min(distances, key=lambda i: (-distances[i], eligible[i]['row']['path'])))
    for n, i in enumerate(chosen, 1):
        x = eligible[i]
        songs.append(dict(id=f'heldout-{n:02d}', published=False, pcm=str(args.private / 'pcm' / f'heldout-{n:02d}.f32'), sample_rate=44100, audio=x['audio'], metadata=x['row'], selection_coordinates=coordinates[i], selection_bpm=x['bpm'], selection_span=x['span'], selection_density=x['density']))
    for song in songs: song['prepared'] = str(args.private / 'prepared' / (song['id'] + '.bin'))
    manifest = dict(corpus_sha256=digest(corpus), eligible_recordings=len(eligible), created_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), seeds=SEEDS, songs=songs)
    dump(dest, manifest)
    print(json.dumps(dict(manifest_sha256=digest(dest), eligible=len(eligible), songs=len(songs), bpm_range=[min(eligible[i]['bpm'] for i in chosen),max(eligible[i]['bpm'] for i in chosen)], span_range=[min(eligible[i]['span'] for i in chosen),max(eligible[i]['span'] for i in chosen)], density_range=[min(eligible[i]['density'] for i in chosen),max(eligible[i]['density'] for i in chosen)])))


class Worker:
    def __init__(self, args):
        self.process = subprocess.Popen([str(args.binary), str(args.onnx), str(args.assets)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
    def call(self, command):
        self.process.stdin.write(json.dumps(command) + '\n'); self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line: raise RuntimeError(f'worker exited {self.process.poll()}')
        result = json.loads(line)
        if 'error' in result: raise RuntimeError(result['error'])
        return result['ok']
    def close(self):
        self.process.stdin.close(); self.process.wait()


def grid_edges(segments, duration):
    # Grid::whole_beats: continuous time, integer origins at segment boundaries.
    offsets = [s[0] for s in segments]
    lengths = [s[1] / 2**math.ceil(math.log2(80 / (60000 / s[1]))) for s in segments]
    origins = [0]
    for i in range(1, len(offsets)):
        origins.append(origins[-1] + math.floor((offsets[i]-offsets[i-1])/lengths[i-1] + 0.5))
    def beat(t):
        i = max(0, bisect.bisect_right(offsets,t)-1)
        return origins[i]+(t-offsets[i])/lengths[i]
    b0 = math.floor(beat(0))
    nb = max(1, math.ceil(beat(duration))-b0)
    edges=[]
    for b in range(b0,b0+nb+1):
        i=max(0,bisect.bisect_right(origins,b)-1)
        edges.append(offsets[i]+(b-origins[i])*lengths[i])
    return edges, beat


def structural(objects, duration, segments):
    # Each tuple is (lane, head_ms, optional tail_ms). A row counts heads, not held lanes.
    rows = collections.Counter(t for _,t,_ in objects)
    heads = sorted(rows)
    lanes = collections.Counter(l for l,_,_ in objects)
    reattacks=[]
    for lane in range(4):
        times=sorted(t for l,t,_ in objects if l==lane)
        reattacks.extend(b-a for a,b in zip(times,times[1:]))
    edges, beat = grid_edges(segments, duration)
    occupied = {max(0,min(len(edges)-2,bisect.bisect_right(edges,t+0.501)-1)) for t in heads}
    gap_times = [0] + heads + [duration]
    return dict(head_rows=len(rows), notes=len(objects), empty_beats=1-len(occupied)/(len(edges)-1), nps=len(objects)/(duration/1000), triples=sum(n==3 for n in rows.values())/max(1,len(rows)), quads=sum(n==4 for n in rows.values())/max(1,len(rows)), ln_share=sum(e is not None and e>t for _,t,e in objects)/max(1,len(objects)), min_reattack=min(reattacks) if reattacks else None, lane_max=max(lanes.values(),default=0)/max(1,len(objects)), lanes_used=len(lanes), max_gap_beats=max((beat(b)-beat(a) for a,b in zip(gap_times,gap_times[1:])),default=0))


def measure(path, duration, calc):
    receipt = json.loads(Path(str(path)+'.receipt.json').read_text())
    parsed=calc.parse_osu_file(str(path)+'.osu')
    # Use the exported file both for star and structural measurements.
    objects=[]
    in_objects=False
    for line in Path(str(path)+'.osu').read_text().splitlines():
        if line == '[HitObjects]': in_objects=True; continue
        if in_objects and line.strip():
            fields=line.split(','); hold=int(fields[3])&128
            objects.append((min(3,int(fields[0])*4//512),float(fields[2]),float(fields[5].split(':')[0]) if hold else None))
    m=structural(objects,duration,receipt['rows']['segments'])
    m['star']=calc.compute_mania_star_rating_20241007(parsed.hit_objects,4,1.0)
    d=receipt['controls']['difficulty']
    m.update(fallback_per_1000=1000*receipt['fallback_count']/max(1,receipt['rows']['K']), fallback_count=receipt['fallback_count'], multiplier=d['m_final'], multiplier_clamped=d['clamped'], q=d['q'], q_untested=not 0.1<=d['q']<=0.75, r=d['achieved_r'], rows_request=receipt['rows']['request'], seconds=receipt['seconds']['total'], hit_objects_sha256=hashlib.sha256(json.dumps(objects).encode()).hexdigest(), osu_sha256=digest(str(path)+'.osu'))
    return m


def candidate(args, worker, song, star, seed, opts, calc):
    # Only after Q1 has proved the plateaus. The search trace still records the literal request
    # and counts every logical generation; this only reuses a byte-identical result.
    if getattr(args, 'cache_baseline_plateaus', False) and not opts:
        star = max(2.125, min(5.875, star))
    identity = dict(star_request_input=float(star),seed=seed,options=opts)
    key=hashlib.sha256(json.dumps(identity,sort_keys=True).encode()).hexdigest()[:20]
    path=args.private/'candidates'/song['id']/key
    saved=Path(str(path)+'.metrics.json')
    if saved.exists(): return json.loads(saved.read_text())
    path.parent.mkdir(parents=True,exist_ok=True)
    try:
        response=worker.call(dict(op='generate',prepared=song['prepared'],star=star,seed=seed,options=opts,out=str(path)))
        value=dict(song=song['id'], **identity, **measure(path,response['song_ms'],calc))
    except RuntimeError as e: value=dict(song=song['id'],**identity,error=str(e))
    dump(saved,value)
    return value


def validate(args, calc):
    values={}
    expected={'adventure':4.63,'catch-the-moment':3.57,'hesitation-snow':2.73,'saigetsu':3.19}
    for name in PUBLISHED:
        path=args.model_repo/'artifacts/runtime-ref/bulk'/name/'reference/generated.osu'
        p=calc.parse_osu_file(path)
        v=calc.compute_mania_star_rating_20241007(p.hit_objects,4,1.0)
        values[name]=v
        assert round(v,2)==expected[name],(name,v,expected[name])
    objects=[(0,0,None),(1,500,None),(2,1000,1250),(3,1500,None),(0,1500,None),(1,1500,None)]
    m=structural(objects,2000,[[0,500,4]])
    assert m['head_rows']==4 and m['empty_beats']==0 and m['nps']==3 and m['triples']==.25 and m['quads']==0 and m['ln_share']==1/6 and m['min_reattack']==1000 and m['lanes_used']==4 and m['lane_max']==1/3 and m['max_gap_beats']==1,m
    empty=structural([],2000,[[0,500,4]])
    assert empty['empty_beats']==1 and empty['max_gap_beats']==4 and empty['head_rows']==0
    edges,_=grid_edges([[0,500,4],[1250,400,4]],2500)
    assert edges==[0,500,1000,1250,1650,2050,2450,2850],edges
    dump(args.private/'measurement-validation.json',dict(published_stars=values,calculator_sha256=digest(args.private/'difficulty_r2_1_0_1.py'),synthetic='passed; hand-counted chord/LN/reattack/rests plus fractional segment boundary'))
    print(json.dumps(values),flush=True)


def prepare_song(args,song):
    if Path(song['prepared']).exists(): return song['id']
    if not Path(song['pcm']).exists():
        subprocess.run(['ffmpeg','-v','error','-i',song['audio'],'-ac','1','-ar',str(song['sample_rate']),'-f','f32le',song['pcm']],check=True)
    Path(song['prepared']).parent.mkdir(parents=True,exist_ok=True)
    w=Worker(args)
    try:
        result=w.call(dict(op='prepare',pcm=song['pcm'],sample_rate=song['sample_rate'],prepared=song['prepared']))
        dump(args.private/'prepared'/(song['id']+'.json'),result)
    finally: w.close()
    return song['id']


def sweep_song(args,song,calc,arms):
    w=Worker(args); results=[]
    try:
        for arm in arms:
            for star in (args.requests or (GRID if arm=='baseline' or args.full_grid else ARM_GRID)):
                for seed in SEEDS:
                    value=candidate(args,w,song,star,seed,arm_options(arm,star),calc)
                    results.append(dict(arm=arm,**value))
            print(f"finished {song['id']} {arm}",flush=True)
    finally: w.close()
    dump(args.private/'sweeps'/args.run/(song['id']+'.json'),results)
    return results


def arm_options(arm,star):
    if arm=='baseline' or 2.125<=star<=5.875: return {}
    low=star<2.125
    f=min(1,max(0,(2.125-star)/1.125 if low else (star-5.875)/3.125))
    opts={}
    for part in arm.split('+'):
        if part in ('overall','end_segment'): opts.update(rows=part,rows_min=0.,rows_max=6.)
        elif part in ('q_tested','q_outer'):
            base=.5 if low else .467
            endpoint=(.1 if low else .75) if part=='q_tested' else (0. if low else .95)
            opts['difficulty_q']=endpoint if f==1 else base+f*(endpoint-base)
        elif part=='ceiling': opts['multiplier_max']=1.2+.1*f
        elif part=='direct':
            # A blend to a direct multiplier, so the baseline endpoint is exactly preserved.
            opts.update(multiplier_target=.8 if low else 1.3,multiplier_blend=f)
        elif part=='masks':
            opts['reattack_floor_ms']=(145+f*(200-145)) if low else (75+f*(50-75))
            if not low: opts['quad_run_cap']=1+f
        else: raise ValueError(arm)
    return opts


def search(generate, target, seed, rule, n=6, tol=.05):
    tries=[]; r=max(2.125,min(5.875,target)) if rule=='bounded' else target; current_seed=seed; saturated=False; seed_tries=0
    for k in range(n):
        value=generate(r,current_seed)
        if 'error' in value: raise RuntimeError(value['error'])
        m=value['star']; tries.append(dict(k=k,r=r,seed=current_seed,m=m))
        best=min(tries,key=lambda x:abs(x['m']-target))
        if rule!='seeds' and (abs(best['m']-target)<=tol or k==n-1): break
        if rule=='seeds': current_seed=seed+k+1; continue
        if not saturated:
            prev=next((t for t in reversed(tries[:-1]) if t['seed']==current_seed and t['r']!=r),None)
            if prev and abs(m-prev['m'])<.01: saturated=True
            else:
                slope=max(.5,min(2,(m-prev['m'])/(r-prev['r']))) if prev and rule in ('secant','bounded') else 1.
                nxt=max(2.125,min(5.875,r+(target-m)/slope)) if rule=='bounded' else max(target-3,min(target+3,r+(target-m)/slope))
                if abs(nxt-r)<.01: saturated=True
                else: r=nxt
        if saturated:
            r=best['r']; seed_tries+=1; current_seed=seed+seed_tries
    assert abs(best['m']-target)<=abs(tries[0]['m']-target)
    return dict(target=target,seed=seed,rule=rule,first_error=abs(tries[0]['m']-target),error=abs(best['m']-target),generations=len(tries),saturated=saturated,best=best,tries=tries)


def search_song(args,song,calc):
    dest=args.private/'search'/args.run/(song['id']+'.json')
    previous=json.loads(dest.read_text()) if dest.exists() else []
    done={(v['target'],v['seed'],v['rule']) for v in previous}
    w=Worker(args); results=previous
    try:
        for target in TARGETS:
            for seed in SEEDS:
                for rule in args.rules.split(','):
                    if (target,seed,rule) in done: continue
                    result=search(lambda r,s:candidate(args,w,song,r,s,arm_options(args.search_arm,r),calc),target,seed,rule)
                    results.append(dict(song=song['id'],build=args.search_arm,**result))
                    dump(dest,results)
            print(f"search {song['id']} target {target}",flush=True)
    finally: w.close()
    return results


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command',choices=['select','validate','prepare','sweep','search'])
    p.add_argument('--model-repo',type=Path,required=True)
    p.add_argument('--private',type=Path,required=True)
    p.add_argument('--binary',type=Path,default=Path('target/release/examples/star-reach'))
    p.add_argument('--onnx',type=Path)
    p.add_argument('--assets',type=Path,default=Path('assets/resampler'))
    p.add_argument('--jobs',type=int,default=2)
    p.add_argument('--run',default='baseline')
    p.add_argument('--arms',default='baseline')
    p.add_argument('--full-grid',action='store_true')
    p.add_argument('--requests',type=lambda s:sorted(set(map(float,s.split(',')))),help='explicit comma-separated sweep requests')
    p.add_argument('--search-arm',default='baseline')
    p.add_argument('--rules',default='secant,seeds,proportional')
    p.add_argument('--songs',help='comma-separated manifest ids, otherwise all')
    p.add_argument('--cache-baseline-plateaus',action='store_true',help='search only: require Q1 equality, then memoize identical endpoint charts')
    args=p.parse_args()
    if any(r not in ['secant','seeds','proportional','bounded'] for r in args.rules.split(',')): p.error('unknown search rule')
    if args.requests and not all(math.isfinite(r) for r in args.requests): p.error('--requests must be finite')
    args.private=args.private.resolve(); args.model_repo=args.model_repo.resolve()
    args.binary=args.binary.resolve(); args.assets=args.assets.resolve()
    args.onnx=args.onnx or args.model_repo/'artifacts/release-r2/bulk/hf/ensomi-r2/onnx'
    args.private.mkdir(parents=True,exist_ok=True)
    if args.command=='select': select(args); return
    calc=load_calculator(args)
    if args.command=='validate': validate(args,calc); return
    manifest=json.loads((args.private/'manifest.json').read_text())
    songs=[s for s in manifest['songs'] if not args.songs or s['id'] in args.songs.split(',')]
    if args.cache_baseline_plateaus:
        assert args.command == 'search' and args.search_arm == 'baseline'
        for song in songs:
            records=json.loads((args.private/'sweeps/baseline'/(song['id']+'.json')).read_text())
            assert len(records)==len(GRID)*len(SEEDS)
            for seed in SEEDS:
                for low in [True,False]:
                    group=[r for r in records if r['seed']==seed and ((r['star_request_input']<=2.125) if low else (r['star_request_input']>=5.875))]
                    assert len({r['hit_objects_sha256'] for r in group})==1
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        if args.command=='prepare': fs=[pool.submit(prepare_song,args,s) for s in songs]
        elif args.command=='sweep': fs=[pool.submit(sweep_song,args,s,calc,args.arms.split(',')) for s in songs]
        else: fs=[pool.submit(search_song,args,s,calc) for s in songs]
        for f in concurrent.futures.as_completed(fs):
            f.result()
    print('complete',args.command,args.run,flush=True)

if __name__=='__main__': main()
