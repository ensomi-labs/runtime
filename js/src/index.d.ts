import type * as ORT from 'onnxruntime-web';

/** EnsomiProtocol 0.2 chart.proto field names and enum values. */
export interface ChartObject {
    lane: number;
    start_ms: number;
    end_ms?: number;
}
export interface Chart {
    duration_ms: number;
    objects: ChartObject[];
}
export interface AudioAssetRef {
    local_path?: string;
    uri?: string;
    asset_id?: string;
    audio_length_ms?: number;
    sha256_hex?: string;
}
export declare enum LongNoteAmount {
    UNSPECIFIED = 0,
    TAP_ONLY = 1,
    HALF = 2,
    NATURAL = 3
}
export declare enum PatternKind {
    UNSPECIFIED = 0,
    STREAM = 1,
    JUMPSTREAM = 2,
    CHORDSTREAM = 3,
    JACK = 4,
    CHORDJACK = 5
}
export declare enum ControlStatus {
    UNSPECIFIED = 0,
    WORKS = 1,
    EXPERIMENTAL = 2,
    NOT_WORKING = 3
}
export declare enum GenerationPhase {
    UNSPECIFIED = 0,
    LOADING = 1,
    TIMING = 2,
    ROWS = 3,
    ARRANGEMENT = 4,
    EXPORT = 5
}
export interface SectionRequest {
    start_ms: number;
    end_ms: number;
    star?: number;
    pattern?: {
        kind: PatternKind;
        strength: number;
    };
}
export interface ChartControls {
    long_notes?: LongNoteAmount;
    sections?: SectionRequest[];
}
export interface GenerateChartRequest {
    audio?: AudioAssetRef;
    star: number;
    seed?: number | bigint;
    controls?: ChartControls;
    title?: string;
    artist?: string;
}
export interface ModelCapabilities {
    model_name: string;
    model_version: string;
    files: {
        path: string;
        sha256_hex: string;
    }[];
    min_star: number;
    max_star_exclusive: number;
    controls: {
        name: string;
        status: ControlStatus;
        note: string;
    }[];
}
export interface GenerationProgress {
    phase: GenerationPhase;
}
export interface GenerationReceipt {
    model_name: string;
    model_version: string;
    seed: number | bigint;
    star: number;
    receipt_json: string;
    osz_path?: string;
}
export interface GenerationResult {
    chart: Chart;
    receipt: GenerationReceipt;
    osu: string;
    osz?: Uint8Array;
}
export interface AudioPCM {
    /** Interleaved samples at the sampleRate passed to generateChart. With both frontend streams it may be the
     * `mel` array itself, as `decodeAudio` gives it; the core then takes that array once. */
    samples: Float32Array | Float64Array;
    channels: number;
    /** Optional already-decoded frontend streams: mono, 24000 and 22050 Hz. */
    mel?: Float32Array;
    beatthis?: Float32Array;
}
export type PCM = Float32Array | Float64Array | AudioPCM;
export interface DecodedAudio {
    pcm: AudioPCM;
    sampleRate: number;
}
export interface EncodedAudio {
    name: string;
    bytes: Uint8Array;
}

export type Ort = typeof ORT;
export type FileSource = (relativePath: string) => Promise<Uint8Array>;
/** Reads files relative to a base URL, with `fetch` or, for a `file:` URL in Node, from the file system. */
export declare function urlSource(baseUrl: string): FileSource;
export declare function localFilesSource(files: Iterable<File>): FileSource;

export declare class Models {
    readonly capabilities: ModelCapabilities;
    readonly ort: Ort;
    readonly sessions: Record<string, ORT.InferenceSession>;
    dispose(): Promise<void>;
}
export interface LoadModelsOptions {
    ort?: Ort;
    session?: ORT.InferenceSession.SessionOptions;
    backend?: 'webgpu' | 'wasm';
    /** ORT WebAssembly settings applied before sessions are created, e.g. wasmPaths and numThreads. */
    wasm?: Partial<ORT.Env.WebAssemblyFlags>;
    /** Reads the `resample-*` tables, needed for raw PCM at a rate other than 22050 Hz without a `beatthis` stream.
     * Default: the package's `resampler/` directory. */
    runtimeSource?: FileSource;
    /** The Wasm core: bytes, a `WebAssembly.Module` or a URL. Default: the package's `pkg/ensomi_generate_wasm_bg.wasm`. */
    core?: BufferSource | WebAssembly.Module | URL | string;
}
export declare function loadModels(source: FileSource | string, options?: LoadModelsOptions): Promise<Models>;

export interface GenerateOptions {
    onProgress?: (progress: GenerationProgress) => void;
    audio?: EncodedAudio;
    setting?: string;
    /** Milliseconds from any fixed origin; the receipt's timings come from it. Default `performance.now`. */
    clock?: () => number;
    /** Recorded draws of a release recording, for conformance checks. */
    replay?: unknown;
}
export declare function generateChart(models: Models, pcm: PCM, sampleRate: number, request: GenerateChartRequest, options?: GenerateOptions): Promise<GenerationResult>;

/** What the stages that read only the audio keep of a song; any request and seed generate from it. */
export declare class Prepared {
    /** The runtime's stored form, for a cache keyed by the audio file's hash. */
    bytes(): Uint8Array;
    /** Reads stored bytes; throws for bytes of another format, runtime version or model. */
    static fromBytes(bytes: Uint8Array): Promise<Prepared>;
}
export interface PrepareOptions {
    onProgress?: (progress: GenerationProgress) => void;
    clock?: () => number;
}
/** Runs the stages that read only the audio: PCM preparation, the spectrogram, BeatThis, the features and the beat grid. */
export declare function prepare(models: Models, pcm: PCM, sampleRate: number, options?: PrepareOptions): Promise<Prepared>;

/** A token of a committed page: a tap, or a hold as its start and, possibly in a later page, its end. */
export interface ChartToken {
    lane: number;
    time_ms: number;
    kind: 'tap' | 'hold_start' | 'hold_end';
}
/** Part of the chart that will not change: every token at or before `coverage_ms`, after the previous page's. */
export interface ChartPage {
    tokens: ChartToken[];
    coverage_ms: number;
    end_of_stream: boolean;
}
export interface ScheduleOptions extends Omit<GenerateOptions, 'onProgress'> {
    /** 'ahead-of-play' (default): a page per head row with R2's difficulty check off, play while generating.
     * 'whole-chart': the whole chart with the check, in one page, before play. */
    mode?: 'ahead-of-play' | 'whole-chart';
    /** The judge of the rules whose settle lead the lead takes; default 'C'. */
    judge?: 'A' | 'B' | 'C' | 'D' | 'E';
    /** How far ahead of the heard time the playfield draws notes, in milliseconds. */
    renderLeadMs?: number;
    /** Room for the variance of the generation's speed on this device, in milliseconds. */
    marginMs?: number;
    /** Each committed page, from the join point of the last `start` on; a new `start` sends them again. */
    onPage?: (page: ChartPage) => void;
}
export type ScheduleSource = Prepared | { pcm: PCM; sampleRate: number };
export type SchedulerAction = { kind: 'ready'; joinMs: number } | { kind: 'done'; result?: GenerationResult };
/**
 * Generation for play. Play may start at t0 once c0 - t0 >= L + max(0, (E - c0)(p/r - 1)): the chart committed through
 * c0, the lead L = max(settle lead, renderLeadMs) + marginMs, the song's end E, the playback rate p and the measured
 * generation speed r. From a position other than 0, play joins at the first time at or after it plus L where no hold
 * is open; earlier objects are not sent.
 */
export declare class ChartScheduler {
    static create(models: Models, source: ScheduleSource, request: GenerateChartRequest, options?: ScheduleOptions): Promise<ChartScheduler>;
    start(atMs: number): void;
    heard(chartMs: number, wallMs: number): void;
    stop(): void;
    /** Runs the graphs until play may start or nothing is left to do; waits for the next event while far enough ahead. */
    work(): Promise<SchedulerAction>;
    /** Committed chart milliseconds per wall millisecond of work so far. */
    readonly rate: number | undefined;
    prepared(): Prepared | undefined;
    free(): void;
}
export declare function decodeAudio(arrayBuffer: ArrayBuffer): Promise<DecodedAudio>;
export declare function validateChart(chart: Chart): void;
