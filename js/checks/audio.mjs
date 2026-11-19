// Wasm audio probe, mono decode and 16-bit WAV transcode checks. No model needed.
import assert from 'node:assert/strict';
import { decodeAudioBytes, decodeAudioWav, probeAudio } from '../src/index.js';

function stereoWav(frames, rate) {
    const dataBytes = frames * 4, bytes = new Uint8Array(44 + dataBytes), view = new DataView(bytes.buffer);
    const putText = (at, text) => [...text].forEach((c, i) => { bytes[at + i] = c.charCodeAt(0); });
    putText(0, 'RIFF');
    view.setUint32(4, 36 + dataBytes, true);
    putText(8, 'WAVEfmt ');
    view.setUint32(16, 16, true);
    view.setUint16(20, 1, true);
    view.setUint16(22, 2, true);
    view.setUint32(24, rate, true);
    view.setUint32(28, rate * 4, true);
    view.setUint16(32, 4, true);
    view.setUint16(34, 16, true);
    putText(36, 'data');
    view.setUint32(40, dataBytes, true);
    for (let i = 0; i < frames; i++) {
        view.setInt16(44 + i * 4, 8192, true);
        view.setInt16(44 + i * 4 + 2, -4096, true);
    }
    return bytes;
}

function monoAiff(frames, rate) {
    // 54-byte AIFF: FORM + COMM (18 bytes, 80-bit IEEE 754 extended rate) + SSND (8 + frames * 2).
    const ssndSize = 8 + frames * 2, formSize = 4 + 26 + 8 + ssndSize, bytes = new Uint8Array(8 + formSize), view = new DataView(bytes.buffer);
    const putText = (at, text) => [...text].forEach((c, i) => { bytes[at + i] = c.charCodeAt(0); });
    putText(0, 'FORM');
    view.setUint32(4, formSize, false);
    putText(8, 'AIFFCOMM');
    view.setUint32(16, 18, false);
    view.setUint16(20, 1, false);
    view.setUint32(22, frames, false);
    view.setUint16(26, 16, false);
    const exp = Math.floor(Math.log2(rate));
    view.setUint16(28, 16383 + exp, false);
    view.setUint32(30, Math.round((rate / 2 ** exp) * 2 ** 31) >>> 0, false);
    view.setUint32(34, 0, false);
    putText(38, 'SSND');
    view.setUint32(42, ssndSize, false);
    view.setUint32(46, 0, false);
    view.setUint32(50, 0, false);
    for (let i = 0; i < frames; i++) view.setInt16(54 + i * 2, 16384, false);
    return bytes;
}

const wav = stereoWav(480, 48000);
const info = await probeAudio(wav, { name: 'track.wav' });
assert.equal(info.sampleRate, 48000);
assert.equal(info.channels, 2);
assert.equal(info.totalFrames, 480);
assert.equal(info.leadFrames, 0);

const decoded = await decodeAudioBytes(wav.buffer, { name: 'track.wav' });
assert.equal(decoded.sampleRate, 48000);
assert.equal(decoded.samples.length, 480);
assert.ok(Math.abs(decoded.durationMs - 10) < 1e-6);
assert.ok(Math.abs(decoded.samples[0] - (8192 - 4096) / 2 / 32768) < 1e-6);

const transcoded = await decodeAudioWav(wav, { name: 'track.wav' });
assert.deepEqual(transcoded, wav);

const aiff = monoAiff(441, 44100);
const aiffDecoded = await decodeAudioBytes(aiff, { name: 'clip.aiff' });
assert.equal(aiffDecoded.sampleRate, 44100);
assert.equal(aiffDecoded.samples.length, 441);
assert.ok(Math.abs(aiffDecoded.samples[0] - 0.5) < 1e-6);

const aiffWav = await decodeAudioWav(aiff, { name: 'clip.aiff' });
const roundtrip = await decodeAudioBytes(aiffWav, { name: 'clip.wav' });
assert.equal(roundtrip.sampleRate, 44100);
assert.equal(roundtrip.samples.length, 441);
assert.ok(Math.abs(roundtrip.samples[0] - 0.5) < 1e-6);

// Valid Ogg Opus stream (OpusHead BOS page + OpusTags EOS page): recognized container with an unsupported codec -> code === 'UNSUPPORTED_AUDIO'.
function oggPage(payload, headerType, seq) {
    const page = new Uint8Array(27 + 1 + payload.length), view = new DataView(page.buffer);
    [...'OggS'].forEach((c, i) => { page[i] = c.charCodeAt(0); });
    page[5] = headerType;
    view.setUint32(14, 1, true);
    view.setUint32(18, seq, true);
    page[26] = 1;
    page[27] = payload.length;
    page.set(payload, 28);
    let crc = 0;
    for (const b of page) {
        crc ^= b << 24;
        for (let bit = 0; bit < 8; bit++) crc = (crc & 0x80000000) ? ((crc << 1) ^ 0x04c11db7) : (crc << 1);
    }
    view.setUint32(22, crc >>> 0, true);
    return page;
}
const opusHead = new Uint8Array(19), opusTags = new Uint8Array(16), opusAudio = new Uint8Array([0xf8, 0xff, 0xfe]);
[...'OpusHead'].forEach((c, i) => { opusHead[i] = c.charCodeAt(0); });
opusHead[8] = 1;
opusHead[9] = 2;
new DataView(opusHead.buffer).setUint32(12, 48000, true);
[...'OpusTags'].forEach((c, i) => { opusTags[i] = c.charCodeAt(0); });
const page0 = oggPage(opusHead, 0x02, 0), page1 = oggPage(opusTags, 0x00, 1), page2 = oggPage(opusAudio, 0x04, 2);
const oggOpus = new Uint8Array(page0.length + page1.length + page2.length);
oggOpus.set(page0, 0);
oggOpus.set(page1, page0.length);
oggOpus.set(page2, page0.length + page1.length);
await assert.rejects(decodeAudioBytes(oggOpus, { name: 'song.opus' }), err => err?.code === 'UNSUPPORTED_AUDIO');
await assert.rejects(decodeAudioBytes(new Uint8Array(0)), /nonempty/);

console.log(JSON.stringify({ passed: true, wav_frames: decoded.samples.length, aiff_frames: aiffDecoded.samples.length }));
