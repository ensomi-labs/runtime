use ensomi_osz::osz;
use std::io::{Cursor, Read};
use zip::{CompressionMethod, ZipArchive};

const OSU: &str = "osu file format v14\n\n[General]\nAudioFilename:audio.mp3\nMode: 3\n\n[HitObjects]\n64,192,1000,1,0,0:0:0:0:\n";

/// Each entry's name, compression and contents, read back by another zip implementation.
fn entries(archive: &[u8]) -> Vec<(String, CompressionMethod, Vec<u8>)> {
    let mut zip = ZipArchive::new(Cursor::new(archive)).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut file = zip.by_index(i).unwrap();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            (file.name().unwrap().to_string(), file.compression(), data)
        })
        .collect()
}

#[test]
fn holds_the_chart_the_song_and_the_receipt() {
    let audio: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let archive = osz(OSU, &audio, r#"{"seed":7}"#);

    assert_eq!(
        entries(&archive),
        [
            ("generated.osu".to_string(), CompressionMethod::Deflated, OSU.as_bytes().to_vec()),
            ("audio.mp3".to_string(), CompressionMethod::Stored, audio.clone()),
            ("receipt.json".to_string(), CompressionMethod::Deflated, b"{\"seed\":7}\n".to_vec()),
        ]
    );
    assert_eq!(archive, osz(OSU, &audio, r#"{"seed":7}"#), "the same inputs give the same bytes");
}

#[test]
fn names_the_audio_after_the_osu() {
    let names = |osu: &str| entries(&osz(osu, b"RIFF", "{}")).into_iter().map(|(name, _, _)| name).collect::<Vec<_>>();
    assert_eq!(names("[General]\nAudioFilename: audio.ogg\n"), ["generated.osu", "audio.ogg", "receipt.json"]);
    assert_eq!(names("[General]\nMode: 3\n"), ["generated.osu", "audio.bin", "receipt.json"]);
}
