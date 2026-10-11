// EnsomiProtocol 0.2 enum values, with the reverse mapping TypeScript numeric enums have.
const numeric = (names) => Object.freeze(Object.fromEntries(names.flatMap((name, value) => [[name, value], [value, name]])));

export const LongNoteAmount = numeric(['UNSPECIFIED', 'TAP_ONLY', 'HALF', 'NATURAL']);
export const PatternKind = numeric(['UNSPECIFIED', 'STREAM', 'JUMPSTREAM', 'CHORDSTREAM', 'JACK', 'CHORDJACK']);
export const ControlStatus = numeric(['UNSPECIFIED', 'WORKS', 'EXPERIMENTAL', 'NOT_WORKING']);
export const GenerationPhase = numeric(['UNSPECIFIED', 'LOADING', 'TIMING', 'ROWS', 'ARRANGEMENT', 'EXPORT']);
