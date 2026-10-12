import { zipSync, strToU8 } from 'fflate';

/** Throws unless the chart's objects are sorted, in lanes 0 to 3, whole milliseconds and without overlap. */
export function validateChart(chart) {
    if (!Number.isInteger(chart.duration_ms) || chart.duration_ms < 0)
        throw new Error('Chart duration must be nonnegative whole milliseconds');
    const end = [-1, -1, -1, -1];
    let previous;
    for (const object of chart.objects) {
        const { lane, start_ms: start, end_ms: release } = object;
        if (!Number.isInteger(lane) || lane < 0 || lane > 3 || !Number.isInteger(start) || start < 0 || start > chart.duration_ms)
            throw new Error(`Invalid chart object: ${JSON.stringify(object)}`);
        if (previous && (start < previous.start_ms || start === previous.start_ms && lane <= previous.lane))
            throw new Error('Chart objects must be sorted by start then lane, with no duplicate heads');
        if (start < end[lane])
            throw new Error(`Head inside an open hold in lane ${lane} at ${start}`);
        if (release !== undefined && (!Number.isInteger(release) || release <= start || release > chart.duration_ms))
            throw new Error(`Invalid hold release in lane ${lane} at ${start}`);
        end[lane] = release ?? start;
        previous = object;
    }
}

export function audioFilename(name) {
    const ext = /\.([a-zA-Z0-9]+)$/.exec(name)?.[1].toLowerCase() ?? 'bin';
    return `audio.${ext}`;
}

export function exportOsz(osu, audio, receiptJson) {
    return zipSync({ 'generated.osu': strToU8(osu), [audioFilename(audio.name)]: [audio.bytes, { level: 0 }], 'receipt.json': strToU8(receiptJson + '\n') }, { level: 6 });
}
