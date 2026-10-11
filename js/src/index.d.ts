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
    /** Interleaved samples at the sampleRate passed to generateChart. */
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
    /** Reads the `resample-*` tables, needed for raw PCM at a rate other than 22050 Hz without a `beatthis` stream. */
    runtimeSource?: FileSource;
    /** The Wasm core: bytes, a `WebAssembly.Module` or a URL. Node cannot fetch the default, the file beside the bindings. */
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
export declare function decodeAudio(arrayBuffer: ArrayBuffer): Promise<DecodedAudio>;
export declare function validateChart(chart: Chart): void;
