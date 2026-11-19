import { callCore, loadCore } from './core.js';

function asBytes(bytes) {
    if (bytes instanceof Uint8Array && bytes.byteLength > 0)
        return bytes;
    if (bytes instanceof ArrayBuffer && bytes.byteLength > 0)
        return new Uint8Array(bytes);
    throw new Error('Audio must be a nonempty Uint8Array or ArrayBuffer');
}

/** Probes an audio file's sample rate, channels, duration, tags, codec and lead frames with the Wasm core (Symphonia). */
export async function probeAudio(bytes, options = {}) {
    const input = asBytes(bytes), core = await loadCore(options.core);
    return callCore(() => core.probeAudio(input, options.name));
}

/** Decodes an audio file to mono `Float32Array` at its own sample rate with BASS / release-engine lead alignment. */
export async function decodeAudioBytes(bytes, options = {}) {
    const input = asBytes(bytes), core = await loadCore(options.core);
    return callCore(() => core.decodeAudioBytes(input, options.name));
}

/** Decodes an audio file to a 16-bit PCM RIFF WAVE (`Uint8Array`) with BASS / release-engine lead alignment. */
export async function decodeAudioWav(bytes, options = {}) {
    const input = asBytes(bytes), core = await loadCore(options.core);
    return callCore(() => core.decodeAudioWav(input, options.name));
}
