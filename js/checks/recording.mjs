// Reader for ensomi-runtime-reference-v1 recordings: index.json and the arrays.bin it addresses.
import { open, readFile } from 'node:fs/promises';
import { join } from 'node:path';

const types = { '<f4': Float32Array, '<f8': Float64Array, '<i8': BigInt64Array, '<i4': Int32Array, '<i2': Int16Array, '<f2': Uint16Array, '|b1': Uint8Array, '|u1': Uint8Array };

export class Recording {
    constructor(index, file) {
        this.index = index;
        this.file = file;
    }
    static async open(directory) {
        const index = JSON.parse(await readFile(join(directory, 'index.json'), 'utf8'));
        if (index.format !== 'ensomi-runtime-reference-v1')
            throw new Error('Unsupported reference format');
        return new Recording(index, await open(join(directory, 'arrays.bin'), 'r'));
    }
    async get(id) {
        const e = this.index.arrays[id], bytes = new Uint8Array(e.nbytes);
        await this.file.read(bytes, 0, e.nbytes, e.byte_offset);
        return new types[e.dtype](bytes.buffer);
    }
    shape(id) { return this.index.arrays[id].shape; }
    async tensor(id, ort) {
        const e = this.index.arrays[id], type = { '<f4': 'float32', '<i8': 'int64', '|b1': 'bool' }[e.dtype];
        return new ort.Tensor(type, await this.get(id), e.shape);
    }
    async event(name) {
        const e = this.index.events.find(x => x.name === name);
        return Object.fromEntries(await Promise.all(Object.entries(e.arrays).map(async ([k, v]) => [k, await this.get(v)])));
    }
    close() { return this.file.close(); }
}

export function difference(actual, expected) {
    let max_abs = 0, index = -1, nonfinite_equal = true, sse = 0;
    for (let i = 0; i < expected.length; i++) {
        const a = Number(actual[i]), b = Number(expected[i]);
        if (!Number.isFinite(a) || !Number.isFinite(b)) {
            if (!(Object.is(a, b) || Number.isNaN(a) && Number.isNaN(b)))
                nonfinite_equal = false;
            continue;
        }
        const delta = Math.abs(a - b);
        sse += delta * delta;
        if (delta > max_abs) {
            max_abs = delta;
            index = i;
        }
    }
    return { max_abs, rms: Math.sqrt(sse / expected.length), index, nonfinite_equal, reference: index < 0 ? null : Number(expected[index]), actual: index < 0 ? null : Number(actual[index]), same_length: actual.length === expected.length };
}

export function firstTextDifference(actual, expected) {
    const a = actual.split('\n'), b = expected.split('\n');
    for (let i = 0; i < Math.max(a.length, b.length); i++)
        if (a[i] !== b[i])
            return { line: i + 1, actual: a[i] ?? null, reference: b[i] ?? null };
    return null;
}

export function parseOsu(text) {
    return text.split('[HitObjects]\n')[1].trim().split('\n').filter(Boolean).map(line => {
        const p = line.split(','), lane = (Number(p[0]) - 64) / 128, start_ms = Number(p[2]);
        return Number(p[3]) & 128 ? { lane, start_ms, end_ms: Number(p[5].split(':')[0]) } : { lane, start_ms };
    });
}

function timingRange(lines) {
    const start = lines.indexOf('[TimingPoints]') + 1;
    let end = start;
    while (start > 0 && /^-?\d/.test(lines[end] ?? ''))
        end++;
    return [start, end];
}

export function withoutTimingPoints(text) {
    const lines = text.split('\n'), [start, end] = timingRange(lines);
    return [...lines.slice(0, start), ...lines.slice(end)].join('\n');
}

export function timingDifference(actual, expected) {
    const rows = (text) => { const lines = text.split('\n'), [start, end] = timingRange(lines); return lines.slice(start, end).map(line => line.split(',').map(Number)); };
    const a = rows(actual), b = rows(expected);
    let max_abs_ms = 0;
    for (let i = 0; i < Math.min(a.length, b.length); i++)
        for (let j = 0; j < b[i].length; j++)
            max_abs_ms = Math.max(max_abs_ms, Math.abs(a[i][j] - b[i][j]));
    return { rows: b.length, same_rows: a.length === b.length && a.every((r, i) => r.length === b[i].length), max_abs_ms };
}

/** The generator comment line differs by runtime name and is not part of the chart. */
export const stripGenerator = (text) => text.split('\n').filter(line => !/^\/\/ (rows=|@ensomi\/runtime |ensomi-runtime )/.test(line)).join('\n');

/** Ties to even, as the core's `round`. */
const round = (x) => {
    const lo = Math.floor(x), d = x - lo;
    return d === 0.5 ? lo + (lo % 2 === 0 ? 0 : 1) : d > 0.5 ? lo + 1 : lo;
};

/** The hit objects of a run of decisions, per row the lane codes and gap releases (export.rs `chart_from_decisions`). */
export function chartFromDecisions(headMs, songMs, actions, gaps) {
    const objects = [], open = [null, null, null, null];
    for (let k = 0; k < actions.length; k++) {
        const time = k < headMs.length ? headMs[k] : songMs;
        for (let lane = 0; lane < 4; lane++) {
            const code = actions[k][lane], wasHeld = open[lane] !== null;
            if (wasHeld && code !== 0) {
                const upper = (k < headMs.length ? headMs[k] : Math.ceil(songMs)) - 1;
                objects[open[lane]].end_ms = code === 1 ? time : Math.max(headMs[k - 1] + 1, Math.min(upper, round(gaps[k][lane])));
                open[lane] = null;
            }
            const tap = wasHeld ? code === 3 : code === 1, hold = wasHeld ? code === 4 : code === 2;
            if (tap || hold) {
                objects.push(hold ? { lane, start_ms: time, end_ms: NaN } : { lane, start_ms: time });
                if (hold)
                    open[lane] = objects.length - 1;
            }
        }
    }
    return objects.sort((a, b) => a.start_ms - b.start_ms || a.lane - b.lane);
}

/** A chart as the tokens of its pages, in their order: by time, a hold's end before a start, then by lane. */
export function chartTokens(chart) {
    const tokens = [];
    for (const o of chart.objects) {
        tokens.push({ lane: o.lane, time_ms: o.start_ms, kind: o.end_ms === undefined ? 'tap' : 'hold_start' });
        if (o.end_ms !== undefined)
            tokens.push({ lane: o.lane, time_ms: o.end_ms, kind: 'hold_end' });
    }
    return tokens.sort((a, b) => a.time_ms - b.time_ms || (a.kind !== 'hold_end') - (b.kind !== 'hold_end') || a.lane - b.lane);
}

/** Whether pages make the chart: one per head row and the final row, rising coverage, one end of stream at the end. */
export function pagesMakeChart(pages, chart, heads) {
    const tokens = pages.flatMap(p => p.tokens);
    const rising = pages.every((p, i) => i === 0 || pages[i - 1].coverage_ms < p.coverage_ms);
    const ends = pages.at(-1)?.end_of_stream === true && pages.filter(p => p.end_of_stream).length === 1;
    const passed = pages.length === heads + 1 && rising && ends && JSON.stringify(tokens) === JSON.stringify(chartTokens(chart));
    return { passed, pages: pages.length, tokens: tokens.length, coverage_rising: rising, one_end_of_stream: ends };
}
