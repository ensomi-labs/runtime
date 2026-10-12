//! The `.osz` archive of a generated chart, with the layout the web runtime gave it: `generated.osu`, the song's
//! audio and `receipt.json`. The caller passes the bytes in and saves the archive; nothing here reads or writes a file.

use miniz_oxide::deflate::compress_to_vec;

/// A zip archive of `generated.osu`, the song's audio and `receipt.json` (the receipt JSON and a newline), in that
/// order. The audio takes the name the `.osu` gives it in its `AudioFilename:` line (`audio.bin` when it has none)
/// and is stored; the other two are deflated. Entries carry the zip format's earliest date, 1980-01-01, so the same
/// inputs give the same bytes.
pub fn osz(osu: &str, audio: &[u8], receipt_json: &str) -> Vec<u8> {
    let audio_name = osu
        .lines()
        .find_map(|line| line.strip_prefix("AudioFilename:"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("audio.bin");
    let receipt = format!("{receipt_json}\n");
    let entries: [(&str, &[u8], bool); 3] =
        [("generated.osu", osu.as_bytes(), true), (audio_name, audio, false), ("receipt.json", receipt.as_bytes(), true)];

    let mut archive = Vec::with_capacity(osu.len() + audio.len() + receipt.len() + 512);
    let mut directory = Vec::new();
    for (name, data, deflate) in entries {
        let compressed = if deflate { compress_to_vec(data, 6) } else { Vec::new() };
        let body = if deflate { &compressed[..] } else { data };
        // The local header and the directory entry share these fields, from "version needed" to "extra field length".
        let mut fields = Vec::with_capacity(26);
        put16(&mut fields, 20);
        put16(&mut fields, if name.is_ascii() { 0 } else { 1 << 11 }); // bit 11: the name is UTF-8
        put16(&mut fields, if deflate { 8 } else { 0 });
        put16(&mut fields, 0); // time 00:00:00
        put16(&mut fields, 1 << 5 | 1); // date 1980-01-01
        put32(&mut fields, crc32fast::hash(data));
        put32(&mut fields, body.len() as u32);
        put32(&mut fields, data.len() as u32);
        put16(&mut fields, name.len() as u16);
        put16(&mut fields, 0);

        let offset = archive.len() as u32;
        put32(&mut archive, 0x0403_4b50);
        archive.extend_from_slice(&fields);
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(body);

        put32(&mut directory, 0x0201_4b50);
        put16(&mut directory, 20); // made by: MS-DOS attributes, version 2.0
        directory.extend_from_slice(&fields);
        put16(&mut directory, 0); // comment length
        put16(&mut directory, 0); // disk number
        put16(&mut directory, 0); // internal attributes
        put32(&mut directory, 0); // external attributes
        put32(&mut directory, offset);
        directory.extend_from_slice(name.as_bytes());
    }

    let directory_offset = archive.len() as u32;
    archive.extend_from_slice(&directory);
    put32(&mut archive, 0x0605_4b50);
    put16(&mut archive, 0); // this disk
    put16(&mut archive, 0); // disk of the directory
    put16(&mut archive, entries.len() as u16);
    put16(&mut archive, entries.len() as u16);
    put32(&mut archive, directory.len() as u32);
    put32(&mut archive, directory_offset);
    put16(&mut archive, 0); // comment length
    archive
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
