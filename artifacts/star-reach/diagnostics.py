#!/usr/bin/env python3
"""Additional aggregate diagnostics and selected published-song examples; no private identities emitted."""
import argparse,collections,hashlib,json,math,statistics
from pathlib import Path
from run import dump,PUBLISHED
from analyze import pct,stats,error_stats,load_sweeps,table,fmt,violations


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
        slope=max(.5,min(2,(t['m']-prev['m'])/(t['r']-prev['r']))) if prev and rule in ('secant','bounded') else 1
        nxt=max(2.125,min(5.875,t['r']+(target-t['m'])/slope)) if rule=='bounded' else max(target-3,min(target+3,t['r']+(target-t['m'])/slope))
        if abs(nxt-t['r'])<.01:
            return 'model_boundary' if rule=='bounded' else ('request_bound' if nxt in (target-3,target+3) else 'small_update')
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
    for path in (a.private/'search').glob('*/*.json'):
        searches += [r for r in json.loads(path.read_text()) if r.get('build',path.parent.name)=='baseline']
    if not searches:
        dump(a.out/'aggregate-diagnostics.json',result);return
    groups=collections.defaultdict(dict)
    for r in searches:groups[r['song'],r['target'],r['seed']][r['rule']]=r
    result['search_cases']=len(searches);result['search_logical_generations']=sum(r['generations'] for r in searches)
    assert all(r['error']<=r['first_error'] and r['generations']<=6 for r in searches)
    for group in groups.values():
        assert len({r['first_error'] for r in group.values()})==1
    bounds=json.loads((a.private/'structural-envelope.json').read_text())
    def warnings(row,trial):
        identity=dict(star_request_input=float(max(2.125,min(5.875,trial['r']))),seed=trial['seed'],options={})
        key=hashlib.sha256(json.dumps(identity,sort_keys=True).encode()).hexdigest()[:20]
        m=json.loads((a.private/'candidates'/row['song']/(key+'.metrics.json')).read_text())
        return violations(m,bounds)
    winner_warnings=[]
    for scope in ['all','interior']:
        for rule in ['secant','seeds','proportional','bounded']:
            rs=[r for r in searches if r['rule']==rule and (scope=='all' or 2.5<=r['target']<=5.5)]
            flags=[(bool(warnings(r,r['tries'][0])),bool(warnings(r,r['best']))) for r in rs]
            winner_warnings.append(dict(scope=scope,rule=rule,n=len(rs),first_flags=sum(x for x,y in flags),winner_flags=sum(y for x,y in flags),new_flags=sum(not x and y for x,y in flags)))
    result['search_winner_structural_warnings']=winner_warnings
    comparisons=[]
    for scope in ['all','interior','outside']:
        for alternative in ['proportional','seeds','bounded']:
            pairs=[(g['secant'],g[alternative]) for (_,target,_),g in groups.items() if 'secant' in g and alternative in g and (scope=='all' or (scope=='interior' and 2.5<=target<=5.5) or (scope=='outside' and (target<2.5 or target>5.5)))]
            if not pairs:continue
            ds=[b['error']-s['error'] for s,b in pairs]
            comparisons.append(dict(scope=scope,alternative=alternative,n=len(pairs),paired_delta_median=pct(ds,.5),paired_delta_mean=statistics.mean(ds),wins=sum(d<-1e-12 for d in ds),ties=sum(abs(d)<=1e-12 for d in ds),losses=sum(d>1e-12 for d in ds),secant=error_stats([s for s,b in pairs]),alternative_stats=error_stats([b for s,b in pairs])))
    result['search_comparisons']=comparisons
    budgets=[]
    for rule in ['secant','proportional','seeds','bounded']:
        for n in [1,2,3,4,5,6]:
            for scope in ['all','interior']:
                rows=[r for r in searches if r['rule']==rule and (scope=='all' or 2.5<=r['target']<=5.5)]
                errors=[min(abs(t['m']-r['target']) for t in r['tries'][:n]) for r in rows]
                budgets.append(dict(rule=rule,N=n,scope=scope,n=len(rows),median=pct(errors,.5),p90=pct(errors,.9),within_005=statistics.mean(e<=.05 for e in errors),within_015=statistics.mean(e<=.15 for e in errors),mean_generations=statistics.mean(min(n,r['generations']) for r in rows)))
    result['prefix_budgets']=budgets
    saturation={}
    for rule in ['secant','proportional','bounded']:
        rows=[r for r in searches if r['rule']==rule]
        causes=collections.Counter(saturation_reason(r) for r in rows)
        assert sum(c for k,c in causes.items() if k!='not_saturated')==sum(r['saturated'] for r in rows)
        saturation[rule]=dict(causes)
    result['saturation_causes']=saturation
    slopes=[]
    for rule in ['secant','proportional','bounded']:
        pairs=[]
        for row in searches:
            if row['rule']!=rule:continue
            for i,t in enumerate(row['tries']):
                prev=next((v for v in reversed(row['tries'][:i]) if v['seed']==t['seed'] and v['r']!=t['r']),None)
                if prev and 2.125<=t['r']<=5.875 and 2.125<=prev['r']<=5.875:
                    pairs.append(((t['m']-prev['m'])/(t['r']-prev['r']),abs(t['m']-prev['m']),abs(t['r']-prev['r'])))
        slopes.append(dict(rule=rule,n=len(pairs),slope=stats([v[0] for v in pairs]),negative=sum(v[0]<0 for v in pairs),flat_under_001=sum(v[1]<.01 for v in pairs),outside_slope_clamp=sum(not .5<=v[0]<=2 for v in pairs)))
    result['within_table_trajectory_slopes']=slopes
    interior=[g for (song,target,seed),g in groups.items() if song in PUBLISHED and 2.5<=target<=5.5 and all(k in g for k in ['secant','proportional','seeds','bounded'])]
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
    lines.append('Same-seed trajectory steps with both requests inside the fitted table (a selected set of steps, not a random derivative sample):\n')
    lines.append(table(['Rule','Steps','Slope median [p10,p90]','Negative slopes','Star Δ < .01','Slopes outside [.5,2]'],[[r['rule'],r['n'],f"{fmt(r['slope']['median'])} [{fmt(r['slope']['p10'])},{fmt(r['slope']['p90'])}]",r['negative'],r['flat_under_001'],r['outside_slope_clamp']] for r in slopes]))
    lines.append('The search optimizes star only. Applying the already fixed structural screen to selected winners is characterization, not a validated playability test; counts below are target/seed cases, with repeated charts.\n')
    lines.append(table(['Scope','Rule','n','First chart warnings','Winner warnings','New warnings'],[[r['scope'],r['rule'],r['n'],r['first_flags'],r['winner_flags'],r['new_flags']] for r in winner_warnings]))
    lines.append('### Published examples selected to expose search failures\n')
    for e in examples:
        lines.append(f"{e['selection']}: **{e['song']}**, target {e['target']}, starting seed {e['starting_seed']}. These cases were selected after measurement to explain failures.\n")
        lines.append(table(['Rule','Try','Requested','Seed','Measured','Absolute error'],[[rule,t['k']+1,fmt(t['r'],4),t['seed'],fmt(t['m'],4),fmt(abs(t['m']-e['target']),4)] for rule,row in e['rules'].items() for t in row['tries']]))
    lines.append('## Workload r and measured star\n')
    lines.append('These deltas pool 24 paired charts, unlike the proposal gate, which first takes the two-seed median within each of 12 songs. Negative star deltas mean easier measured output; opposite signs show why runtime r cannot stand in for star.\n')
    lines.append(table(['Arm','End','Median star Δ','Median r Δ','Opposite sign / nonzero pairs'],[[r['arm'],r['end'],fmt(r['median_star_delta']),fmt(r['median_r_delta']),f"{r['opposite_signs']}/{r['nonzero_pairs']}"] for r in signs]))
    (a.out/'diagnostics.md').write_text('\n'.join(lines))
    print(json.dumps({k:v for k,v in result.items() if k in ['physical_stored_candidates','baseline_observed_star_range','search_cases','search_logical_generations','saturation_causes']}))

if __name__=='__main__':main()
