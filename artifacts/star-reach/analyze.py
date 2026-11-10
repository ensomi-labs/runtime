#!/usr/bin/env python3
"""Aggregate private results; --out receives ONLY aggregates, never song-level records."""
from __future__ import annotations
import argparse, collections, csv, hashlib, json, math, statistics
from pathlib import Path
from run import dump, GRID, TARGETS, SEEDS

METRICS=['star','fallback_per_1000','empty_beats','nps','triples','quads','ln_share','min_reattack','lane_max','max_gap_beats','multiplier','q','r','rows_request','seconds']

def pct(values,q):
    x=sorted(values)
    if not x: return None
    p=(len(x)-1)*q; i=math.floor(p)
    return x[i]+(x[min(i+1,len(x)-1)]-x[i])*(p-i)


def stats(x):
    return dict(median=pct(x,.5),p10=pct(x,.1),p90=pct(x,.9),min=min(x) if x else None,max=max(x) if x else None)


def song_medians(rows,key='star'):
    groups=collections.defaultdict(list)
    for r in rows:
        if 'error' not in r: groups[r['song']].append(r[key])
    return {k:statistics.median(v) for k,v in groups.items()}


def seed_spread(rows):
    groups=collections.defaultdict(dict)
    for r in rows:
        if 'error' not in r: groups[r['song']][r['seed']]=r['star']
    return [abs(v[SEEDS[0]]-v[SEEDS[1]]) for v in groups.values() if all(s in v for s in SEEDS)]


def envelope(rows):
    refs=collections.defaultdict(list)
    for r in rows:
        if r['star_request_input'] in (2.0,5.875): refs[r['song']].append(r)
    bounds={}
    for song, group in refs.items():
        assert len(group)==4,(song,len(group))
        def mm(key):return min(r[key] for r in group),max(r[key] for r in group)
        def upper(key,delta):return [0,min(1,mm(key)[1]+delta)]
        bounds[song]={'fallback_per_1000':[0,max(5,2*mm('fallback_per_1000')[1])], 'empty_beats':upper('empty_beats',.15),'nps':[.5*mm('nps')[0],1.5*mm('nps')[1]],'triples':upper('triples',.1),'quads':upper('quads',.1),'ln_share':[max(0,mm('ln_share')[0]-.1),min(1,mm('ln_share')[1]+.1)],'min_reattack':[max(50,.8*mm('min_reattack')[0]),None],'lane_max':[0,max(.45,mm('lane_max')[1]+.1)],'max_gap_beats':[0,max(8,2*mm('max_gap_beats')[1])],'multiplier':[.8,1.3]}
    return bounds


def violations(r,bounds):
    if 'error' in r:return ['generation_failed']
    bad=[]
    if r['head_rows']<2: bad.append('too_few_rows')
    if r['lanes_used']!=4: bad.append('unused_lane')
    for key,(lo,hi) in bounds[r['song']].items():
        x=r[key]
        if x is None or x<lo-1e-9 or (hi is not None and x>hi+1e-9): bad.append(key)
    return bad


def load_sweeps(root):
    values=[]
    for p in sorted((root/'sweeps').glob('*/*.json')):
        values.extend(json.loads(p.read_text()))
    # Sweep reruns and full-grid arms share candidates: count each arm/song/request/seed once.
    return list({(r['arm'],r['song'],r['star_request_input'],r['seed']):r for r in values}.values())


def aggregate(rows):
    successful=[r for r in rows if 'error' not in r]
    out=dict(n=len(rows),failed=len(rows)-len(successful),song_median=statistics.median(song_medians(rows).values()) if successful else None)
    for key in METRICS:
        for s,v in stats([r[key] for r in successful if r[key] is not None]).items():out[key+'_'+s]=v
    for key in ['multiplier_clamped','q_untested']:
        out[key+'_share']=statistics.mean(r[key] for r in successful) if successful else None
    return out


def fmt(x,d=3):
    return '—' if x is None else f'{x:.{d}f}'


def table(header,rows):
    return '\n'.join(['| '+' | '.join(header)+' |','| '+' | '.join(['---']*len(header))+' |']+['| '+' | '.join(map(str,r))+' |' for r in rows])+'\n'


def error_stats(rows,key='error'):
    x=[r[key] for r in rows]
    return dict(n=len(x),median=pct(x,.5),p90=pct(x,.9),within_005=statistics.mean(v<=.05 for v in x),within_015=statistics.mean(v<=.15 for v in x),generations=1 if key=='first_error' else statistics.mean(r['generations'] for r in rows),saturated=None if key=='first_error' else statistics.mean(r['saturated'] for r in rows))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--private',type=Path,required=True)
    p.add_argument('--out',type=Path,default=Path('artifacts/star-reach'))
    p.add_argument('--freeze-envelope',action='store_true')
    a=p.parse_args(); a.out.mkdir(parents=True,exist_ok=True)
    rows=load_sweeps(a.private)
    baseline=[r for r in rows if r['arm']=='baseline']
    assert len(baseline)==12*2*len(GRID),len(baseline)
    bounds=envelope(baseline)
    bound_path=a.private/'structural-envelope.json'
    if a.freeze_envelope:
        if bound_path.exists(): assert json.loads(bound_path.read_text())==bounds
        else: dump(bound_path,bounds)
    else: assert json.loads(bound_path.read_text())==bounds
    per_request=collections.defaultdict(list)
    for r in rows:per_request[(r['arm'],r['star_request_input'])].append(r)
    summaries=[]
    for (arm,request),group in sorted(per_request.items()):
        bad=collections.Counter(k for r in group for k in violations(r,bounds))
        summaries.append(dict(arm=arm,request=request,**aggregate(group),failed_envelope=sum(bool(violations(r,bounds)) for r in group),violations=dict(bad)))
    dump(a.out/'aggregate-reach.json',summaries)
    flat=[{k:v for k,v in r.items() if k!='violations'} for r in summaries]
    with (a.out/'aggregate-reach.csv').open('w') as f:
        writer=csv.DictWriter(f,fieldnames=list(flat[0]));writer.writeheader();writer.writerows(flat)
    lines=['## Q1: baseline measurements','', 'All star ratings below are measurements, not playability judgements. The centre is the median of 12 song medians; p10–p90 and extrema pool 24 charts. Seed spread is the median paired absolute difference between seeds 0 and 1000.','']
    q1=[]
    for request in GRID:
        group=per_request['baseline',request]; v=aggregate(group); spread=stats(seed_spread(group))
        q1.append([fmt(request,4),fmt(v['song_median']),f"{fmt(v['star_p10'])}–{fmt(v['star_p90'])}",f"{fmt(v['star_min'])}–{fmt(v['star_max'])}",fmt(spread['median']),fmt(spread['p90'])])
    lines.append(table(['Request','Song median','Chart p10–p90','Chart min–max','Seed Δ median','Seed Δ p90'],q1))
    edge_groups={end:per_request['baseline',r] for end,r in [('low',2.),('high',5.875)]}
    null={end:statistics.median(seed_spread(group)) for end,group in edge_groups.items()}
    plateau={end:statistics.median(song_medians(group).values()) for end,group in edge_groups.items()}
    plateau_checks={}
    for end,grid in [('low',[r for r in GRID if r<=2.125]),('high',[r for r in GRID if r>=5.875])]:
        ref={(r['song'],r['seed']):r['hit_objects_sha256'] for r in edge_groups[end]}
        plateau_checks[end]=all(r['hit_objects_sha256']==ref[r['song'],r['seed']] for q in grid for r in per_request['baseline',q])
    interior=[r for r in baseline if 2.125<=r['star_request_input']<=5.875]
    xs=[r['star_request_input'] for r in interior]; ys=[r['star'] for r in interior]
    xm,ym=statistics.mean(xs),statistics.mean(ys)
    slope=sum((x-xm)*(y-ym) for x,y in zip(xs,ys))/sum((x-xm)**2 for x in xs)
    lines.append(f"Hit-object plateau equality: {plateau_checks}. In-table OLS slope {slope:.3f}, intercept {ym-slope*xm:.3f}; mean measured-minus-request bias {statistics.mean(y-x for x,y in zip(xs,ys)):.3f}, median bias {statistics.median(y-x for x,y in zip(xs,ys)):.3f}. These are descriptive across this selected panel, without a population confidence claim.\n")
    lines.extend(['## Q2: arms and structural references','', 'The baseline fitted-edge rows are included below. Each cell gives a median unless marked maximum or minimum. F is fallbacks/1,000 head rows (median/max); E is empty-beat share; 3/4 are the separate three-/four-head row shares; LN is long-note head share; A is the smallest same-lane reattack across all 24 charts in ms; L is maximum-lane share (median); G is longest head-free gap in beats (median). V counts charts violating at least one per-song envelope; C/Q are the shares with multiplier clamp / untested quantile. The JSON/CSV companions include p10, p90, minimum and maximum for every metric, plus workload r, workload request, multiplier, q, timing and failure counts.',''])
    for arm in sorted({r['arm'] for r in rows}):
        selected=[s for s in summaries if s['arm']==arm and (arm!='baseline' or s['request'] in (2.,5.875))]
        lines.append(f'### {arm}\n')
        lines.append(table(['Request','Star med [p10,p90]','F med/max','E','NPS','3 / 4','LN','A min','L','G','V','C / Q'],[[fmt(s['request'],4),f"{fmt(s['song_median'])} [{fmt(s['star_p10'])},{fmt(s['star_p90'])}]",f"{fmt(s['fallback_per_1000_median'])}/{fmt(s['fallback_per_1000_max'])}",fmt(s['empty_beats_median']),fmt(s['nps_median']),f"{fmt(s['triples_median'])}/{fmt(s['quads_median'])}",fmt(s['ln_share_median']),fmt(s['min_reattack_min'],1),fmt(s['lane_max_median']),fmt(s['max_gap_beats_median']),str(s['failed_envelope']),f"{fmt(s['multiplier_clamped_share'],2)}/{fmt(s['q_untested_share'],2)}"] for s in selected]))
    verdicts=[]
    for arm in sorted({r['arm'] for r in rows}-{'baseline'}):
        for end in ['low','high']:
            selected=sorted([s for s in summaries if s['arm']==arm and ((s['request']<=2.125) if end=='low' else (s['request']>=5.875))],key=lambda s:s['request'])
            if not selected:continue
            endpoint=selected[0] if end=='low' else selected[-1]
            candidate_rows=per_request[arm,endpoint['request']]
            reference=song_medians(edge_groups[end]); measured=song_medians(candidate_rows)
            sign=-1 if end=='low' else 1
            movement=statistics.median(sign*(measured[s]-reference[s]) for s in reference) if set(measured)==set(reference) else None
            monotone=all(a['song_median'] is not None and b['song_median'] is not None and b['song_median']>=a['song_median']-1e-9 for a,b in zip(selected,selected[1:]))
            violation_count=sum(s['failed_envelope'] for s in selected if s['request'] not in (2.125,5.875))
            bad=collections.Counter()
            for s in selected:bad.update(s['violations'])
            gate=monotone and movement is not None and movement>=.5 and movement>2*null[end] and violation_count==0
            verdicts.append(dict(arm=arm,end=end,endpoint=endpoint['request'],endpoint_star=endpoint['song_median'],paired_movement=movement,null_spread=null[end],monotone=monotone,envelope_failures=violation_count,indicators=dict(bad),gate=gate))
    dump(a.out/'aggregate-verdicts.json',verdicts)
    interior_centres=[aggregate(per_request['baseline',q])['song_median'] for q in GRID if 2.125<=q<=5.875]
    interior_monotone=all(b>=a-1e-9 for a,b in zip(interior_centres,interior_centres[1:]))
    caps=[]
    for arm in sorted({r['arm'] for r in rows}-{'baseline'}):
        for end in ['low','high']:
            selected=sorted([v for v in summaries if v['arm']==arm and ((v['request']<=2.125) if end=='low' else (v['request']>=5.875))],key=lambda v:v['request'],reverse=end=='low')
            if not selected:continue
            safe=selected[0];stop=None;reason=[]
            sign=-1 if end=='low' else 1
            for v in selected[1:]:
                if v['failed_envelope']:reason.append('envelope')
                if v['song_median'] is None or sign*(v['song_median']-safe['song_median']) < -1e-9:reason.append('median reversal')
                if reason:stop=v['request'];break
                safe=v
            reference=song_medians(edge_groups[end]);measured=song_medians(per_request[arm,safe['request']])
            movement=statistics.median(sign*(measured[k]-reference[k]) for k in reference) if set(measured)==set(reference) else None
            gate=movement is not None and movement>=.5 and movement>2*null[end] and interior_monotone
            dense=all((arm,q) in per_request for q in GRID if (safe['request']<=q<2.125 if end=='low' else 5.875<q<=safe['request']))
            caps.append(dict(arm=arm,end=end,safe_cap=safe['request'],star=safe['song_median'],paired_movement=movement,first_failure=stop,reason=reason,unchanged_interior_monotone=interior_monotone,preliminary_gate=gate,dense_grid_checked=dense))
    dump(a.out/'aggregate-caps.json',caps)
    lines.extend(['### Gate at the tested endpoints','',table(['Arm','End','Median star','Paired movement','2 × null','Monotone','Envelope failures','Pass'],[[v['arm'],v['end'],fmt(v['endpoint_star']),fmt(v['paired_movement']),fmt(2*v['null_spread']),v['monotone'],v['envelope_failures'],v['gate']] for v in verdicts])])
    lines.extend(['### Contiguous cap check','',table(['Arm','End','Last safe request','Median star','Paired movement','First failed request','Why stopped','Preliminary gate'],[[v['arm'],v['end'],fmt(v['safe_cap'],4),fmt(v['star']),fmt(v['paired_movement']),str(v['first_failure']),', '.join(v['reason']) or 'test limit',v['preliminary_gate']] for v in caps])])
    lines.append(f"Unchanged in-table baseline medians are non-decreasing: {interior_monotone}. A preliminary cap pass still needs the denser grid; this table cannot alone authorize a default.\n")
    lines.extend(['Failures count repeated candidates at different requests; the private envelope file contains every song-specific bound. Passing this structural screen would still not establish playability. A full-grid/cap assessment is required before proposing a default.',''])
    searches=[]
    for path in sorted((a.private/'search').glob('*/*.json')):
        for r in json.loads(path.read_text()):searches.append(dict(build=path.parent.name,**r))
    if searches:
        lines.extend(['## Q3: client-side search','', 'Errors are absolute stars; proportions are fractions. Six-seed runs always consume six logical generations. Secant/proportional runs may stop at tolerance. Cached candidates still count as generations.',''])
        search_summaries=[]
        for build in sorted({r['build'] for r in searches}):
            groups=[r for r in searches if r['build']==build]
            for target in TARGETS+['all','interior','outside']:
                rows_t=[r for r in groups if target=='all' or (target=='interior' and 2.5<=r['target']<=5.5) or (target=='outside' and (r['target']<2.5 or r['target']>5.5)) or r['target']==target]
                for rule in ['first','secant','seeds','proportional']:
                    rs=[r for r in rows_t if r['rule']==('secant' if rule=='first' else rule)]
                    if rs:search_summaries.append(dict(build=build,target=target,rule=rule,**error_stats(rs,'first_error' if rule=='first' else 'error')))
            lines.append(f'### {build}\n')
            lines.append(table(['Target','Rule','n','Error median','Error p90','≤.05','≤.15','Mean generations','Saturated'],[[str(s['target']),s['rule'],s['n'],fmt(s['median']),fmt(s['p90']),fmt(s['within_005']),fmt(s['within_015']),('1' if s['rule']=='first' else fmt(s['generations'],2)),('—' if s['rule']=='first' else fmt(s['saturated']))] for s in search_summaries if s['build']==build]))
        dump(a.out/'aggregate-search.json',search_summaries)
    (a.out/'results.md').write_text('\n'.join(lines))
    print(json.dumps(dict(baseline_plateau=plateau,null_spread=null,slope=slope,arms=len(verdicts)//2,search_cases=len(searches))))

if __name__=='__main__':main()
