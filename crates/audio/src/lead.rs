//! Unified BASS / Release Engine timeline alignment for MP3 and AAC.
//!
//! Ports Ensomi's `Mp3Lead` rules (`ensomi-model/src/ensomi_model/audio_rows/decoder.py` and
//! `ensomi-app`'s `Mp3Lead.kt`) and adds MP4/M4A AAC priming extraction (`iTunSMPB` and `elst`) so
//! frame 0 of decoded PCM aligns identically across osu!/BASS, Apple AVFoundation, and Android.
//!
//! Symphonia's MP3 decoder (`MpaDecoder` with `FormatOptions { enable_gapless: false }`) skips any
//! leading Xing/Info/VBRI frame in the demuxer and does **not** trim the 529-frame Layer III
//! synthesis filterbank delay (`decoder_trim = 0`). Symphonia's ISO-MP4 demuxer also leaves AAC
//! priming untrimmed (`elst` and `iTunSMPB` are not applied).

use std::io::{self, Read, Seek, SeekFrom};

/// The Layer III synthesis filterbank decoder delay, in frames, for MPEG-1, 2 and 2.5 alike.
pub const DECODER_DELAY_FRAMES: i32 = 529;

/// How far past an ID3v2 tag the first Layer III frame is looked for, matching the release engine.
pub const SCAN_BYTES: usize = 65_536;

/// What the first MPEG Layer III frame of a file carries, in the classes of the release engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mp3Header {
    /// A Xing or Info header with a LAME extension: its encoder string, encoder delay, and padding.
    Lame {
        encoder: String,
        delay: u32,
        padding: u32,
    },
    /// A Xing or Info header without a LAME extension.
    Xing,
    /// A Fraunhofer VBRI header.
    Vbri,
    /// A Layer III frame with neither header.
    None,
    /// No Layer III frame followed by another in the scanned bytes.
    NoSync,
}

/// The number of frames BASS / the release engine trims from the start of an MP3 with `header`.
pub fn release_engine_trim(header: &Mp3Header) -> i32 {
    match header {
        Mp3Header::Lame { delay, .. } if *delay > 0 => (*delay as i32) + DECODER_DELAY_FRAMES,
        _ => 0,
    }
}

/// The lead in frames for an MP3 whose leading bytes are `bytes` (its ID3v2 tag plus up to
/// [`SCAN_BYTES`] after it), for a decoder that internally trims `decoder_trim` frames from the
/// start (`0` for Symphonia with `enable_gapless: false`, `529` for Android/Apple platform codecs).
///
/// Positive values mean prepend that many silence frames; negative values mean skip that many
/// frames from the start of the decoder's raw output.
pub fn mp3_lead_frames(bytes: &[u8], decoder_trim: i32) -> i32 {
    if let Some(priming) = i_tunes_priming(bytes) {
        return decoder_trim - DECODER_DELAY_FRAMES - (priming as i32);
    }
    let header = mp3_header(bytes);
    mp3_lead_from_header(&header, decoder_trim)
}

pub(crate) fn mp3_lead_from_header(header: &Mp3Header, decoder_trim: i32) -> i32 {
    match header {
        Mp3Header::Lame { .. } | Mp3Header::Xing | Mp3Header::None => {
            decoder_trim - release_engine_trim(header)
        }
        Mp3Header::Vbri | Mp3Header::NoSync => 0,
    }
}

/// How many leading bytes of an MP3 file [`mp3_lead_frames`] needs, given its first 10 bytes.
pub fn bytes_needed(head: &[u8]) -> usize {
    id3v2_bytes(head).saturating_add(SCAN_BYTES)
}

/// The number of bytes an ID3v2 tag at the start of `bytes` occupies (header + payload + optional
/// footer), or `0` if there is no ID3v2 header.
pub fn id3v2_bytes(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    let mut size: usize = 0;
    for &b in &bytes[6..10] {
        size = (size << 7) | usize::from(b & 0x7F);
    }
    let footer = if (bytes[5] & 0x10) != 0 { 10 } else { 0 };
    10usize.saturating_add(size).saturating_add(footer)
}

/// Parses the first Layer III frame (after any ID3v2 tag) in `bytes` into an [`Mp3Header`].
pub fn mp3_header(bytes: &[u8]) -> Mp3Header {
    let start = id3v2_bytes(bytes).min(bytes.len());
    let end = bytes.len().min(start.saturating_add(SCAN_BYTES));
    mp3_header_in_slice(&bytes[start..end])
}

/// Parses the first Layer III frame in a slice that already begins after any ID3v2 tag.
pub(crate) fn mp3_header_in_slice(bytes: &[u8]) -> Mp3Header {
    let end = bytes.len().min(SCAN_BYTES);
    if end < 4 {
        return Mp3Header::NoSync;
    }
    for j in 0..(end - 4) {
        let Some(first) = frame_at(bytes, j, end) else {
            continue;
        };
        if frame_at(bytes, j + first.length, end).is_none() {
            continue;
        }
        if matches_at(bytes, j + 36, end, b"VBRI") {
            return Mp3Header::Vbri;
        }
        let x = j + 4 + first.side_info_bytes;
        if !matches_at(bytes, x, end, b"Xing") && !matches_at(bytes, x, end, b"Info") {
            return Mp3Header::None;
        }
        if x + 8 > end {
            return Mp3Header::Xing;
        }
        let flags = u32::from_be_bytes([bytes[x + 4], bytes[x + 5], bytes[x + 6], bytes[x + 7]]);
        let mut p = x + 8;
        if (flags & 1) != 0 {
            p += 4; // frames
        }
        if (flags & 2) != 0 {
            p += 4; // bytes
        }
        if (flags & 4) != 0 {
            p += 100; // TOC
        }
        if (flags & 8) != 0 {
            p += 4; // quality
        }
        if p + 23 > end || !bytes[p..p + 4].iter().all(|&c| (32..=126).contains(&c)) {
            return Mp3Header::Xing;
        }
        let encoder: String = bytes[p..p + 9]
            .iter()
            .map(|&b| char::from(b))
            .collect::<String>()
            .trim_matches(|c| c == '\0' || c == ' ')
            .to_string();
        let b21 = u32::from(bytes[p + 21]);
        let b22 = u32::from(bytes[p + 22]);
        let b23 = if p + 24 <= end { u32::from(bytes[p + 23]) } else { 0 };
        let delay = (b21 << 4) | (b22 >> 4);
        let padding = ((b22 & 0x0F) << 8) | b23;
        return Mp3Header::Lame { encoder, delay, padding };
    }
    Mp3Header::NoSync
}

/// Extracts the priming frame count (2nd hexadecimal field) from an `iTunSMPB` text value.
pub fn parse_itunsmpb_text(text: &str) -> Option<u32> {
    let fields = text
        .split_whitespace()
        .filter(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_alphanumeric()));
    let mut hex = fields.skip_while(|f| !f.chars().all(|c| c.is_ascii_hexdigit()));
    let _first = hex.next()?;
    let second = hex.next()?;
    if !second.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(second, 16)
        .ok()
        .and_then(|v| u32::try_from(v).ok())
}

/// Extracts the `iTunSMPB` priming from an ID3v2 tag at the start of `bytes`, if present.
pub fn i_tunes_priming(bytes: &[u8]) -> Option<u32> {
    let tag_end = bytes.len().min(id3v2_bytes(bytes));
    let key = b"iTunSMPB";
    if tag_end < key.len() {
        return None;
    }
    let at = bytes[..tag_end].windows(key.len()).position(|w| w == key)?;
    let tail_start = at + key.len();
    let tail_end = tag_end.min(tail_start + 400);
    extract_itunsmpb_from_raw(&bytes[tail_start..tail_end])
}

fn extract_itunsmpb_from_raw(raw: &[u8]) -> Option<u32> {
    let mut text = String::with_capacity(raw.len());
    for &c in raw {
        match c {
            0 => {}
            0x20..=0x7E => text.push(char::from(c)),
            _ => text.push(' '),
        }
    }
    parse_itunsmpb_text(&text)
}

/// Pre-scanned container lead metadata collected from the raw `Read + Seek` stream before handing
/// ownership to Symphonia's `MediaSourceStream`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ContainerLeadHint {
    pub mp3_lead: Option<i32>,
    pub mp4_elst_delay: Option<(u64, u32)>, // (media_time, timescale)
}

/// Inspects the stream's header without consuming it (rewinds to 0 before returning).
///
/// For WAV/FLAC/OGG/MKV, only the first 10 bytes are read. For MP4/M4A, the `moov` box is walked
/// for the audio track's `elst` edit list. For MP3 (or files starting with an ID3v2 tag), the
/// ID3v2 tag is streamed for `iTunSMPB` and up to [`SCAN_BYTES`] after it are scanned for
/// Xing/Info/LAME/VBRI headers.
pub(crate) fn scan_container_lead<R: Read + Seek>(
    reader: &mut R,
    byte_len: Option<u64>,
) -> io::Result<ContainerLeadHint> {
    reader.seek(SeekFrom::Start(0))?;
    let mut head10 = [0u8; 10];
    let n = read_up_to(reader, &mut head10)?;
    let head = &head10[..n];

    let mut hint = ContainerLeadHint::default();

    if n >= 8 && &head[4..8] == b"ftyp" {
        hint.mp4_elst_delay = read_mp4_audio_elst_delay(reader, byte_len)?;
    } else if n >= 4
        && (&head[..4] == b"RIFF"
            || &head[..4] == b"fLaC"
            || &head[..4] == b"OggS"
            || &head[..4] == b"\x1a\x45\xdf\xa3")
    {
        // Known non-MP3, non-MP4 container: no lead scan needed.
    } else {
        // Possible MP3 (starts with ID3v2, MPEG sync, or leading garbage).
        let id3_len = id3v2_bytes(head) as u64;
        let priming = if id3_len > 0 {
            let scan_limit = byte_len.map_or(id3_len, |len| id3_len.min(len));
            find_itunsmpb_in_id3(reader, scan_limit)?
        } else {
            None
        };
        if let Some(p) = priming {
            hint.mp3_lead = Some(-DECODER_DELAY_FRAMES - (p as i32));
        } else {
            reader.seek(SeekFrom::Start(id3_len))?;
            let mut mpeg_buf = vec![0u8; SCAN_BYTES];
            let mpeg_n = read_up_to(reader, &mut mpeg_buf)?;
            let header = mp3_header_in_slice(&mpeg_buf[..mpeg_n]);
            hint.mp3_lead = Some(mp3_lead_from_header(&header, 0));
        }
    }

    reader.seek(SeekFrom::Start(0))?;
    Ok(hint)
}

/// Streams through an ID3v2 tag (`[0..tag_end)`) in bounded chunks looking for `iTunSMPB`.
fn find_itunsmpb_in_id3<R: Read + Seek>(reader: &mut R, tag_end: u64) -> io::Result<Option<u32>> {
    const CHUNK: usize = 65_536;
    const KEY: &[u8] = b"iTunSMPB";
    // Cap absurdly large malformed ID3v2 headers to 64 MiB.
    let limit = tag_end.min(64 * 1024 * 1024);
    reader.seek(SeekFrom::Start(0))?;

    let mut pos: u64 = 0;
    let mut buf = vec![0u8; CHUNK + KEY.len()];
    let mut carry = 0usize;

    while pos < limit {
        let to_read = ((limit - pos) as usize).min(CHUNK);
        let n = read_up_to(reader, &mut buf[carry..carry + to_read])?;
        if n == 0 {
            break;
        }
        let valid = carry + n;
        if let Some(idx) = buf[..valid].windows(KEY.len()).position(|w| w == KEY) {
            let match_abs = pos.saturating_sub(carry as u64) + idx as u64 + KEY.len() as u64;
            let tail_len = limit.saturating_sub(match_abs).min(400) as usize;
            reader.seek(SeekFrom::Start(match_abs))?;
            let mut tail = vec![0u8; tail_len];
            let tn = read_up_to(reader, &mut tail)?;
            return Ok(extract_itunsmpb_from_raw(&tail[..tn]));
        }
        pos += n as u64;
        let keep = valid.min(KEY.len() - 1);
        buf.copy_within(valid - keep..valid, 0);
        carry = keep;
    }

    Ok(None)
}

/// Walks an ISO-MP4 file's `moov` box to extract `(media_time, timescale)` from the first audio
/// track's `edts/elst` atom, if any entry has `media_time > 0`.
fn read_mp4_audio_elst_delay<R: Read + Seek>(
    reader: &mut R,
    byte_len: Option<u64>,
) -> io::Result<Option<(u64, u32)>> {
    let file_end = match byte_len {
        Some(len) => len,
        None => reader.seek(SeekFrom::End(0))?,
    };
    let mut pos = 0u64;
    while let Some(b) = read_box_header(reader, pos, file_end)? {
        if &b.kind == b"moov" {
            return parse_moov_elst(reader, b.content_start, b.end);
        }
        pos = b.end;
    }
    Ok(None)
}

struct Mp4Box {
    kind: [u8; 4],
    content_start: u64,
    end: u64,
}

fn read_box_header<R: Read + Seek>(
    reader: &mut R,
    pos: u64,
    parent_end: u64,
) -> io::Result<Option<Mp4Box>> {
    if pos.saturating_add(8) > parent_end {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(pos))?;
    let mut hdr = [0u8; 8];
    if read_up_to(reader, &mut hdr)? < 8 {
        return Ok(None);
    }
    let size32 = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let kind = [hdr[4], hdr[5], hdr[6], hdr[7]];
    let (content_start, end) = match size32 {
        0 => (pos + 8, parent_end),
        1 => {
            if pos.saturating_add(16) > parent_end {
                return Ok(None);
            }
            let mut ext = [0u8; 8];
            if read_up_to(reader, &mut ext)? < 8 {
                return Ok(None);
            }
            let size64 = u64::from_be_bytes(ext);
            if size64 < 16 {
                return Ok(None);
            }
            (pos + 16, pos.saturating_add(size64).min(parent_end))
        }
        n if n >= 8 => (pos + 8, pos.saturating_add(u64::from(n)).min(parent_end)),
        _ => return Ok(None),
    };
    if end <= pos || end < content_start {
        return Ok(None);
    }
    Ok(Some(Mp4Box { kind, content_start, end }))
}

fn parse_moov_elst<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> io::Result<Option<(u64, u32)>> {
    let mut pos = start;
    while let Some(b) = read_box_header(reader, pos, end)? {
        if &b.kind == b"trak" {
            if let Some(delay) = parse_trak_elst(reader, b.content_start, b.end)? {
                return Ok(Some(delay));
            }
        }
        pos = b.end;
    }
    Ok(None)
}

fn parse_trak_elst<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> io::Result<Option<(u64, u32)>> {
    let mut is_audio = false;
    let mut timescale: Option<u32> = None;
    let mut elst_media_time: Option<u64> = None;

    let mut pos = start;
    while let Some(b) = read_box_header(reader, pos, end)? {
        match &b.kind {
            b"mdia" => {
                let (audio, ts) = parse_mdia(reader, b.content_start, b.end)?;
                is_audio = audio;
                timescale = ts;
            }
            b"edts" => {
                elst_media_time = parse_edts(reader, b.content_start, b.end)?;
            }
            _ => {}
        }
        pos = b.end;
    }

    if is_audio {
        if let (Some(mt), Some(ts)) = (elst_media_time, timescale) {
            if mt > 0 && ts > 0 {
                return Ok(Some((mt, ts)));
            }
        }
    }
    Ok(None)
}

fn parse_mdia<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> io::Result<(bool, Option<u32>)> {
    let mut is_audio = false;
    let mut timescale = None;
    let mut pos = start;
    while let Some(b) = read_box_header(reader, pos, end)? {
        match &b.kind {
            b"mdhd" => {
                timescale = parse_mdhd_timescale(reader, b.content_start, b.end)?;
            }
            b"hdlr" => {
                if parse_hdlr_is_sound(reader, b.content_start, b.end)? {
                    is_audio = true;
                }
            }
            _ => {}
        }
        pos = b.end;
    }
    Ok((is_audio, timescale))
}

fn parse_mdhd_timescale<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> io::Result<Option<u32>> {
    if start + 4 > end {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(start))?;
    let mut vf = [0u8; 4];
    if read_up_to(reader, &mut vf)? < 4 {
        return Ok(None);
    }
    let version = vf[0];
    let skip = match version {
        0 => 8u64,  // creation_time(4) + modification_time(4)
        1 => 16u64, // creation_time(8) + modification_time(8)
        _ => return Ok(None),
    };
    if start + 4 + skip + 4 > end {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(start + 4 + skip))?;
    let mut ts = [0u8; 4];
    if read_up_to(reader, &mut ts)? < 4 {
        return Ok(None);
    }
    Ok(Some(u32::from_be_bytes(ts)))
}

fn parse_hdlr_is_sound<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> io::Result<bool> {
    // version/flags(4) + pre_defined(4) + handler_type(4)
    if start + 12 > end {
        return Ok(false);
    }
    reader.seek(SeekFrom::Start(start + 8))?;
    let mut handler = [0u8; 4];
    if read_up_to(reader, &mut handler)? < 4 {
        return Ok(false);
    }
    Ok(&handler == b"soun")
}

fn parse_edts<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> io::Result<Option<u64>> {
    let mut pos = start;
    while let Some(b) = read_box_header(reader, pos, end)? {
        if &b.kind == b"elst" {
            return parse_elst(reader, b.content_start, b.end);
        }
        pos = b.end;
    }
    Ok(None)
}

fn parse_elst<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> io::Result<Option<u64>> {
    if start + 8 > end {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(start))?;
    let mut hdr = [0u8; 8];
    if read_up_to(reader, &mut hdr)? < 8 {
        return Ok(None);
    }
    let version = hdr[0];
    let entry_count = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]).min(64);
    let entry_size: u64 = match version {
        0 => 12,
        1 => 20,
        _ => return Ok(None),
    };
    let mut pos = start + 8;
    for _ in 0..entry_count {
        if pos + entry_size > end {
            break;
        }
        reader.seek(SeekFrom::Start(pos))?;
        let media_time: i64 = if version == 0 {
            let mut buf = [0u8; 12];
            if read_up_to(reader, &mut buf)? < 12 {
                break;
            }
            i64::from(i32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]))
        } else {
            let mut buf = [0u8; 20];
            if read_up_to(reader, &mut buf)? < 20 {
                break;
            }
            i64::from_be_bytes([
                buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
            ])
        };
        if media_time >= 0 {
            return Ok(Some(media_time as u64));
        }
        pos += entry_size;
    }
    Ok(None)
}

pub(crate) fn scale_to_sample_rate(value: u64, timescale: u32, sample_rate: u32) -> u32 {
    if timescale == 0 || sample_rate == 0 {
        return 0;
    }
    if timescale == sample_rate {
        return u32::try_from(value).unwrap_or(u32::MAX);
    }
    let scaled = (u128::from(value) * u128::from(sample_rate) + u128::from(timescale) / 2)
        / u128::from(timescale);
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

fn read_up_to<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

struct Frame {
    length: usize,
    side_info_bytes: usize,
}

const BITRATES_V1: [usize; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const BITRATES_V2: [usize; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

fn sample_rates(version: u8) -> Option<[usize; 3]> {
    match version {
        3 => Some([44_100, 48_000, 32_000]),
        2 => Some([22_050, 24_000, 16_000]),
        0 => Some([11_025, 12_000, 8_000]),
        _ => None,
    }
}

fn frame_at(b: &[u8], j: usize, end: usize) -> Option<Frame> {
    if j + 4 > end {
        return None;
    }
    let b0 = b[j];
    let b1 = b[j + 1];
    let b2 = b[j + 2];
    let b3 = b[j + 3];
    if b0 != 0xFF || (b1 & 0xE0) != 0xE0 {
        return None;
    }
    let version = (b1 >> 3) & 3;
    let layer = (b1 >> 1) & 3;
    let bitrate_index = usize::from(b2 >> 4);
    let rate_index = usize::from((b2 >> 2) & 3);
    if layer != 1 || bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    let sample_rate = sample_rates(version)?[rate_index];
    let mpeg1 = version == 3;
    let mono = (b3 >> 6) == 3;
    let bitrate = (if mpeg1 { BITRATES_V1 } else { BITRATES_V2 })[bitrate_index] * 1_000;
    let padding = usize::from((b2 >> 1) & 1);
    Some(Frame {
        length: (if mpeg1 { 144 } else { 72 }) * bitrate / sample_rate + padding,
        side_info_bytes: if mpeg1 {
            if mono { 17 } else { 32 }
        } else if mono {
            9
        } else {
            17
        },
    })
}

fn matches_at(b: &[u8], at: usize, end: usize, tag: &[u8]) -> bool {
    at.checked_add(tag.len()).is_some_and(|e| e <= end) && &b[at..at + tag.len()] == tag
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_frame(fill: impl FnOnce(&mut FrameBuilder)) -> Vec<u8> {
        let mut b = FrameBuilder::new([0xFF, 0xFB, 0x90, 0x00], 417, 32);
        fill(&mut b);
        b.bytes
    }

    fn mpeg2_mono_frame(fill: impl FnOnce(&mut FrameBuilder)) -> Vec<u8> {
        let mut b = FrameBuilder::new([0xFF, 0xF3, 0x80, 0xC0], 208, 9);
        fill(&mut b);
        b.bytes
    }

    struct FrameBuilder {
        bytes: Vec<u8>,
        side_info: usize,
    }

    impl FrameBuilder {
        fn new(header: [u8; 4], len: usize, side_info: usize) -> Self {
            let mut bytes = vec![0u8; len];
            bytes[..4].copy_from_slice(&header);
            Self { bytes, side_info }
        }

        fn put(&mut self, at: usize, data: &[u8]) {
            self.bytes[at..at + data.len()].copy_from_slice(data);
        }

        fn xing(&mut self, tag: &[u8; 4], flags: u32, lame: Option<(&str, u32, u32)>) {
            let mut p = 4 + self.side_info;
            self.put(p, tag);
            self.put(p + 4, &flags.to_be_bytes());
            p += 8;
            if (flags & 1) != 0 {
                p += 4;
            }
            if (flags & 2) != 0 {
                p += 4;
            }
            if (flags & 4) != 0 {
                p += 100;
            }
            if (flags & 8) != 0 {
                p += 4;
            }
            if let Some((enc, delay, padding)) = lame {
                let mut enc_buf = [b' '; 9];
                let eb = enc.as_bytes();
                enc_buf[..eb.len().min(9)].copy_from_slice(&eb[..eb.len().min(9)]);
                self.put(p, &enc_buf);
                let b21 = (delay >> 4) as u8;
                let b22 = (((delay & 0xF) << 4) | ((padding >> 8) & 0xF)) as u8;
                let b23 = (padding & 0xFF) as u8;
                self.put(p + 21, &[b21, b22, b23]);
            }
        }
    }

    fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in parts {
            out.extend_from_slice(p);
        }
        out
    }

    fn id3v2(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut body = concat(frames);
        body.extend_from_slice(&[0u8; 64]);
        let size = body.len();
        let mut out = vec![
            b'I',
            b'D',
            b'3',
            3,
            0,
            0,
            ((size >> 21) & 0x7F) as u8,
            ((size >> 14) & 0x7F) as u8,
            ((size >> 7) & 0x7F) as u8,
            (size & 0x7F) as u8,
        ];
        out.extend_from_slice(&body);
        out
    }

    fn comm(description: &str, text: &str) -> Vec<u8> {
        let mut body = vec![0u8];
        body.extend_from_slice(b"eng");
        body.extend_from_slice(description.as_bytes());
        body.push(0);
        body.extend_from_slice(text.as_bytes());
        let size = body.len() as u32;
        let mut out = b"COMM".to_vec();
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn headerless_mp3_lead_rules() {
        let file = concat(&[stereo_frame(|_| {}), stereo_frame(|_| {}), stereo_frame(|_| {})]);
        assert_eq!(mp3_header(&file), Mp3Header::None);
        assert_eq!(mp3_lead_frames(&file, DECODER_DELAY_FRAMES), 529);
        assert_eq!(mp3_lead_frames(&file, 0), 0);
    }

    #[test]
    fn xing_without_lame_lead_rules() {
        let file = concat(&[
            stereo_frame(|f| f.xing(b"Xing", 0x0F, None)),
            stereo_frame(|_| {}),
        ]);
        assert_eq!(mp3_header(&file), Mp3Header::Xing);
        assert_eq!(mp3_lead_frames(&file, DECODER_DELAY_FRAMES), 529);
        assert_eq!(mp3_lead_frames(&file, 0), 0);
    }

    #[test]
    fn lame_delay_trims_delay_plus_decoder_delay() {
        let file = concat(&[
            stereo_frame(|f| f.xing(b"Info", 0x0F, Some(("LAME3.100", 576, 1234)))),
            stereo_frame(|_| {}),
        ]);
        assert_eq!(
            mp3_header(&file),
            Mp3Header::Lame { encoder: "LAME3.100".to_string(), delay: 576, padding: 1234 }
        );
        assert_eq!(release_engine_trim(&mp3_header(&file)), 576 + 529);
        assert_eq!(mp3_lead_frames(&file, DECODER_DELAY_FRAMES), -576);
        assert_eq!(mp3_lead_frames(&file, 0), -(576 + 529));
    }

    #[test]
    fn lame_zero_delay_counts_as_no_delay() {
        let file = concat(&[
            stereo_frame(|f| f.xing(b"Info", 0x0F, Some(("LAME3.99r", 0, 0)))),
            stereo_frame(|_| {}),
        ]);
        assert_eq!(release_engine_trim(&mp3_header(&file)), 0);
        assert_eq!(mp3_lead_frames(&file, DECODER_DELAY_FRAMES), 529);
        assert_eq!(mp3_lead_frames(&file, 0), 0);
    }

    #[test]
    fn mpeg2_mono_shorter_side_info() {
        let file = concat(&[
            mpeg2_mono_frame(|f| f.xing(b"Xing", 0x0F, Some(("LAME3.100", 576, 0)))),
            mpeg2_mono_frame(|_| {}),
        ]);
        assert_eq!(
            mp3_header(&file),
            Mp3Header::Lame { encoder: "LAME3.100".to_string(), delay: 576, padding: 0 }
        );
    }

    #[test]
    fn vbri_and_nosync_leave_as_is() {
        let vbri = concat(&[
            stereo_frame(|f| f.put(36, b"VBRI")),
            stereo_frame(|_| {}),
        ]);
        assert_eq!(mp3_header(&vbri), Mp3Header::Vbri);
        assert_eq!(mp3_lead_frames(&vbri, 0), 0);
        assert_eq!(mp3_lead_frames(&vbri, DECODER_DELAY_FRAMES), 0);

        let noise: Vec<u8> = (0..5_000usize).map(|i| ((i * 7) % 251) as u8).collect();
        assert_eq!(mp3_header(&noise), Mp3Header::NoSync);
        assert_eq!(mp3_lead_frames(&noise, 0), 0);
    }

    #[test]
    fn id3v2_and_itunsmpb_priming() {
        let tag = id3v2(&[comm("iTunSMPB", " 00000000 00000840 000001C0 0000000000ABCDEF 00000000")]);
        let file = concat(&[
            tag.clone(),
            stereo_frame(|f| f.xing(b"Info", 0x0F, Some(("LAME3.100", 576, 0)))),
            stereo_frame(|_| {}),
        ]);
        assert_eq!(i_tunes_priming(&file), Some(0x840));
        assert_eq!(mp3_lead_frames(&file, DECODER_DELAY_FRAMES), -0x840);
        assert_eq!(mp3_lead_frames(&file, 0), -(0x840 + 529));
        assert_eq!(bytes_needed(&file[..10]), tag.len() + SCAN_BYTES);
    }
}
