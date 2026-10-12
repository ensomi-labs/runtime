/** Browser decode on two frontend clocks. WebAudio's codec/resampler can differ from Python's. */
export async function decodeAudio(arrayBuffer) {
    if (!(arrayBuffer instanceof ArrayBuffer) || arrayBuffer.byteLength === 0)
        throw new Error('Audio must be a nonempty ArrayBuffer');
    const decode = async (rate) => {
        const context = new OfflineAudioContext(1, 1, rate), buffer = await context.decodeAudioData(arrayBuffer.slice(0)), mono = new Float32Array(buffer.length);
        for (let channel = 0; channel < buffer.numberOfChannels; channel++) {
            const samples = buffer.getChannelData(channel);
            for (let i = 0; i < mono.length; i++)
                mono[i] += samples[i] / buffer.numberOfChannels;
        }
        return mono;
    };
    const [mel, beatthis] = await Promise.all([decode(24000), decode(22050)]);
    let peak = 0;
    for (const sample of mel)
        peak = Math.max(peak, Math.abs(sample));
    if (peak)
        for (let i = 0; i < mel.length; i++)
            mel[i] /= peak;
    return { pcm: { samples: mel, channels: 1, mel, beatthis }, sampleRate: 24000 };
}
