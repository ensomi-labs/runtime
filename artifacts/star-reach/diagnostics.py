#!/usr/bin/env python3
"""Additional aggregate diagnostics and selected published-song examples; no private identities emitted."""
import argparse,collections,json,math,statistics
from pathlib import Path
from run import dump,PUBLISHED
from analyze import pct,stats,error_stats,load_sweeps,table,fmt


def saturation_reason(row):
    tries=row['tries'];target=row['target'];rule=row['rule']
    if rule=='seeds':return 'not_applicable'
    for i,t in enumerate(tries):
        best=min(tries[:i+1],key=lambda x:abs(x['m']-target))
        if abs(best['m']-target)<=.05 or i==5:break
        prev=next((p for p in reversed(tries[:i]) if p['seed']==t['seed'] and p['r']!=t['r']),None)
        if prev and abs(t['m']-prev['m'])<.01:
            if max(t['r'],prev['r'])<=2.125:return 'lower_plateau'
            if min(t['r'],prev['r'])>=5.875:return 'upper_plateau'
            return 'flat_response_elsewhere'
        slope=max(.5,min(2,(t['m']-prev['m'])/(t['r']-prev['r']))) if prev and rule=='secant' else 1
        nxt=max(target-3,min(target+3,t['r']+(target-t['m'])/slope))
        if abs(nxt-t['r'])<.01:
            return 'request_bound' if nxt in (target-3,target+3) else 'small_update'
    return 'not_saturated'


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--private',type=Path,required=True);p.add_argument('--out',type=Path,default=Path(__file__).parent);a=p.parse_args()
    sweeps=load_sweeps(a.private)
    baseline={(r['song'],r['star_request_input'],r['seed']):r for r in sweeps if r['arm']=='baseline'}
    metrics=[]
    for path in (a.private/'candidates').glob('*/*.metrics.json'):
        metrics.append(json.loads(path.read_text()))
    actual=[r for r in metrics if not r['options'] and 'error' not in r]
    result={'physical_stored_candidates':len(metrics),'baseline_stored_candidates':len(actual),'baseline_observed_star_range':[min(r['star'] for r in actual),max(r['star'] for r in actual)],'baseline_distinct_charts':len({r['hit_objects_sha256'] for r in actual})}
    for name,edge in [('low',2.125),('high',5.875)]:
        rows=[r for r in actual if r['star_request_input']==edge]
        by_song=collections.defaultdict(list)
        for r in rows:by_song[r['song']].append(r['star'])
        result[name+'_sampled_seed_star_range']=stats([r['star'] for r in rows])
        result[name+'_median_song_extreme']=statistics.median((min(v) if name=='low' else max(v)) for v in by_song.values())
    signs=[]
    for arm in sorted({r['arm'] for r in sweeps}-{'baseline'}):
        for end,target in [('low',1.),('high',9.)]:
            rows=[r for r in sweeps if r['arm']==arm and r['star_request_input']==target and 'error' not in r]
            pairs=[(r,baseline[r['song'],target,r['seed']]) for r in rows]
            ds=[r['star']-b['star'] for r,b in pairs];dr=[r['r']-b['r'] for r,b in pairs]
            active=[(s,r) for s,r in zip(ds,dr) if abs(s)>1e-8 and abs(r)>1e-8]
            signs.append(dict(arm=arm,end=end,n=len(pairs),median_star_delta=pct(ds,.5),median_r_delta=pct(dr,.5),nonzero_pairs=len(active),opposite_signs=sum(s*r<0 for s,r in active)))
    result['star_vs_r']=signs
    searches=[]
    for path in (a.private/'search/baseline').glob('*.json'):searches+=json.loads(path.read_text())
    if not searches:
        dump(a.out/'aggregate-diagnostics.json',result);return
    groups=collections.defaultdict(dict)
    for r in searches:groups[r['song'],r['target'],r['seed']][r['rule']]=r
    result['search_cases']=len(searches);result['search_logical_generations']=sum(r['generations'] for r in searches)
    comparisons=[]
    for scope in ['all','interior','outside']:
        for alternative in ['proportional','seeds']:
            pairs=[(g['secant'],g[alternative]) for (_,target,_),g in groups.items() if 'secant' in g and alternative in g and (scope=='all' or (scope=='interior' and 2.5<=target<=5.5) or (scope=='outside' and (target<2.5 or target>5.5)))]
            if not pairs:continue
            ds=[b['error']-s['error'] for s,b in pairs]
            comparisons.append(dict(scope=scope,alternative=alternative,n=len(pairs),paired_delta_median=pct(ds,.5),paired_delta_mean=statistics.mean(ds),wins=sum(d<-1e-12 for d in ds),ties=sum(abs(d)<=1e-12 for d in ds),losses=sum(d>1e-12 for d in ds),secant=error_stats([s for s,b in pairs]),alternative_stats=error_stats([b for s,b in pairs])))
    result['search_comparisons']=comparisons
    budgets=[]
    for rule in ['secant','proportional','seeds']:
        for n in [1,2,3,4,5,6]:
            for scope in ['all','interior']:
                rows=[r for r in searches if r['rule']==rule and (scope=='all' or 2.5<=r['target']<=5.5)]
                errors=[min(abs(t['m']-r['target']) for t in r['tries'][:n]) for r in rows]
                budgets.append(dict(rule=rule,N=n,scope=scope,n=len(rows),median=pct(errors,.5),p90=pct(errors,.9),within_005=statistics.mean(e<=.05 for e in errors),within_015=statistics.mean(e<=.15 for e in errors),mean_generations=statistics.mean(min(n,r['generations']) for r in rows)))
    result['prefix_budgets']=budgets
    saturation={}
    for rule in ['secant','proportional']:
        rows=[r for r in searches if r['rule']==rule]
        causes=collections.Counter(saturation_reason(r) for r in rows)
        assert sum(c for k,c in causes.items() if k!='not_saturated')==sum(r['saturated'] for r in rows)
        saturation[rule]=dict(causes)
    result['saturation_causes']=saturation
    interior=[g for (song,target,seed),g in groups.items() if song in PUBLISHED and 2.5<=target<=5.5 and all(k in g for k in ['secant','proportional','seeds'])]
    # Deliberately selected explanatory cases, not a new decision sample.
    examples=[]
    if interior:
        for reason,group in [('largest published secant miss',max(interior,key=lambda g:g['secant']['error'])),('largest published proportional advantage',max(interior,key=lambda g:g['secant']['error']-g['proportional']['error']))]:
            examples.append(dict(selection=reason,song=group['secant']['song'],target=group['secant']['target'],starting_seed=group['secant']['seed'],rules={k:{f:v[f] for f in ['error','generations','saturated','tries']} for k,v in group.items()}))
    result['published_examples']=examples
    dump(a.out/'aggregate-diagnostics.json',result)
    lines=['## Search diagnostics','', 'The following prefix budgets reuse the same traces, stopping after N tries. They are descriptive checks on this sample, not independently tuned or validated constants.','']
    lines.append(table(['Scope','Rule','N','Median error','p90 error','≤.05','≤.15','Mean used'],[[r['scope'],r['rule'],r['N'],fmt(r['median']),fmt(r['p90']),fmt(r['within_005']),fmt(r['within_015']),fmt(r['mean_generations'],2)] for r in budgets if r['N'] in [3,4,5,6]]))
    lines.append('Saturation causes: `'+json.dumps(saturation,sort_keys=True)+'`.\n')
    lines.append(table(['Scope','Alternative','Wins/ties/losses vs secant','Median paired error Δ','Mean paired error Δ'],[[r['scope'],r['alternative'],f"{r['wins']}/{r['ties']}/{r['losses']}",fmt(r['paired_delta_median']),fmt(r['paired_delta_mean'])] for r in comparisons]))
    lines.append('### Published examples selected to expose search failures\n')
    for e in examples:
        lines.append(f"{e['selection']}: **{e['song']}**, target {e['target']}, starting seed {e['starting_seed']}. These cases were selected after measurement to explain failures.\n")
        lines.append(table(['Rule','Try','Requested','Seed','Measured','Absolute error'],[[rule,t['k']+1,fmt(t['r'],4),t['seed'],fmt(t['m'],4),fmt(abs(t['m']-e['target']),4)] for rule,row in e['rules'].items() for t in row['tries']]))
    lines.append('## Workload r and measured star\n')
    lines.append(table(['Arm','End','Median star Δ','Median r Δ','Opposite sign / nonzero pairs'],[[r['arm'],r['end'],fmt(r['median_star_delta']),fmt(r['median_r_delta']),f"{r['opposite_signs']}/{r['nonzero_pairs']}"] for r in signs]))
    (a.out/'diagnostics.md').write_text('\n'.join(lines))
    print(json.dumps({k:v for k,v in result.items() if k in ['physical_stored_candidates','baseline_observed_star_range','search_cases','search_logical_generations','saturation_causes']}))

if __name__=='__main__':main()
