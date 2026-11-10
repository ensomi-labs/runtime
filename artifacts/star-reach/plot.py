#!/usr/bin/env python3
"""Plot only the public aggregate JSON tables; needs matplotlib."""
import argparse,json
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--aggregates',type=Path,default=Path(__file__).parent)
a=p.parse_args();root=a.aggregates
reach=json.loads((root/'aggregate-reach.json').read_text())
plt.rcParams.update({'font.size':10,'axes.spines.top':False,'axes.spines.right':False,'svg.fonttype':'none'})
fig,ax=plt.subplots(1,2,figsize=(11,4.3),layout='constrained')
b=sorted([r for r in reach if r['arm']=='baseline'],key=lambda r:r['request'])
x=[r['request'] for r in b]
ax[0].fill_between(x,[r['star_p10'] for r in b],[r['star_p90'] for r in b],alpha=.2,color='#0072B2',label='Chart p10–p90 (not a CI)')
ax[0].plot(x,[r['song_median'] for r in b],color='#0072B2',label='Median of song medians')
ax[0].plot([1,9],[1,9],'--',color='gray',lw=1,label='Measured = requested')
ax[0].set_title('Baseline reach across 12 songs × 2 seeds')
ax[0].set_xlabel('Requested star');ax[0].set_ylabel('Measured osu!mania star');ax[0].legend(fontsize=8)
colors=['#0072B2','#D55E00','#009E73','#CC79A7','#E69F00','#56B4E9','#000000','#AA3377','#777777','#44AA99','#882255','#999933']
for i,arm in enumerate(sorted({r['arm'] for r in reach})):
    rows=sorted([r for r in reach if r['arm']==arm],key=lambda r:r['request'])
    groups=[rows] if arm=='baseline' else [[r for r in rows if r['request']<=2.125],[r for r in rows if r['request']>=5.875]]
    labelled=False
    for group in groups:
        if not group:continue
        ax[1].plot([r['request'] for r in group],[r['song_median'] for r in group],label=arm if not labelled else None,color=colors[i%len(colors)],marker='.' if arm!='baseline' else None,lw=1.2)
        labelled=True
ax[1].set_title('Opt-in arms: raw measured reach')
ax[1].set_xlabel('Requested star');ax[1].set_ylabel('Median of song medians')
ax[1].legend(fontsize=7,ncol=2)
for axes in ax:
    axes.axvspan(2.125,5.875,color='gray',alpha=.07)
    axes.set_xlim(1,9);axes.grid(alpha=.15)
fig.suptitle('Star rating and structural screening do not establish playability',fontsize=11)
fig.savefig(root/'reach.png',dpi=170);fig.savefig(root/'reach.svg');plt.close(fig)
if (root/'aggregate-search.json').exists():
    search=json.loads((root/'aggregate-search.json').read_text())
    fig,ax=plt.subplots(1,2,figsize=(11,4.3),layout='constrained')
    for rule,color in zip(['first','secant','seeds','proportional','bounded'],colors):
        rows=sorted([r for r in search if r['build']=='baseline' and r['rule']==rule and isinstance(r['target'],(float,int))],key=lambda r:r['target'])
        ax[0].plot([r['target'] for r in rows],[r['median'] for r in rows],label=rule,color=color,marker='.',lw=1.5)
        ax[1].plot([r['target'] for r in rows],[r['within_005'] for r in rows],label=rule,color=color,marker='.',lw=1.5)
    ax[0].set_ylabel('Median absolute error (stars)');ax[1].set_ylabel('Share within 0.05 stars')
    ax[0].set_title('Error after at most six generations');ax[1].set_title('Tolerance success across 24 cases per target')
    for axes in ax:
        axes.set_xlabel('Target star');axes.set_xlim(1,9);axes.grid(alpha=.15);axes.legend(fontsize=8)
    fig.savefig(root/'search.png',dpi=170);fig.savefig(root/'search.svg');plt.close(fig)
