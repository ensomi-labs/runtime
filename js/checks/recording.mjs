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
