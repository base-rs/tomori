// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Decoding a slot's Ogg Vorbis file a packet at a time, on the ambience's own thread, from its bytes in memory, so
//! that no sound is ever held decoded whole: mono or stereo out, whichever the voice wants, over and over for a loop.
use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use lewton::inside_ogg::OggStreamReader;

type Reader = OggStreamReader<Cursor<Arc<[u8]>>>;

/// The rates a sound may be at, beyond which it is refused rather than played far too fast or never ending.
const RATES: std::ops::RangeInclusive<u32> = 8_000..=192_000;

/// What `decode` gives, or why it cannot: a decoder that panics on a malformed file fails as if it had refused it, so
/// that a bad file silences only its own sound.
pub(super) fn guard<T, E: std::fmt::Display>(
    decode: impl FnOnce() -> Result<T, E>,
) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(decode)) {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err("its decoder failed".to_owned()),
    }
}

fn open(bytes: &Arc<[u8]>) -> Result<Reader, String> {
    guard(|| OggStreamReader::new(Cursor::new(bytes.clone())))
}

/// The frames `bytes` hold, by the granule position of their last whole Ogg page that has one, which says where the
/// stream's audio ends, as decoding trims it: none, for bytes whose pages say nothing.
pub(super) fn frames(bytes: &[u8]) -> u64 {
    let mut frames = 0;
    let mut at = 0;
    while let Some(header) = bytes.get(at..at + 27)
        && header.starts_with(b"OggS")
    {
        let body = at + 27 + usize::from(header[26]);
        let Some(table) = bytes.get(at + 27..body) else {
            break;
        };
        let end = body + table.iter().map(|&size| usize::from(size)).sum::<usize>();
        if end > bytes.len() {
            break;
        }
        let mut granule = [0; 8];
        granule.copy_from_slice(&header[6..14]);
        // A page on which no packet ends says so with all ones.
        let granule = u64::from_le_bytes(granule);
        if granule != u64::MAX {
            frames = granule;
        }
        at = end;
    }
    frames
}

/// One sound being decoded.
pub(crate) struct Decoder {
    bytes: Arc<[u8]>,
    reader: Reader,
    /// The file's channels, of which the first two are heard.
    channels: usize,
    rate: u32,
    /// The packet being read out, interleaved, and how far through it.
    packet: Vec<i16>,
    at: usize,
    /// How many frames this pass through the file has decoded.
    decoded: u64,
    looping: bool,
    /// Whether this pass has given any audio, so that a loop of none ends rather than beginning again for ever.
    heard: bool,
    /// Whether it could not be read on, so that a loop ends there rather than beginning again.
    failed: bool,
}

impl Decoder {
    /// Decode `bytes`, once, or over and over if `looping`. Refused if they are not Ogg Vorbis, or not at a rate from
    /// 8 to 192 kHz.
    pub fn new(bytes: Arc<[u8]>, looping: bool) -> Result<Self, String> {
        let reader = open(&bytes)?;
        let rate = reader.ident_hdr.audio_sample_rate;
        if !RATES.contains(&rate) {
            return Err(format!("its rate, {rate} Hz, is not from 8 to 192 kHz"));
        }
        Ok(Self {
            channels: usize::from(reader.ident_hdr.audio_channels).max(1),
            rate: reader.ident_hdr.audio_sample_rate,
            reader,
            bytes,
            packet: Vec::new(),
            at: 0,
            decoded: 0,
            looping,
            heard: false,
            failed: false,
        })
    }

    /// Whether it could not be read on.
    #[cfg(test)]
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Its samples a second.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// How long its sound lasts once through, at its own rate.
    pub fn length(&self) -> Duration {
        Duration::from_secs_f64(frames(&self.bytes) as f64 / f64::from(self.rate))
    }

    /// Append up to `frames` frames to `out`, in `channels`, one or two, in `i16` units: a mono file heard in both of
    /// two, and a stereo one, or the first two channels of more, mixed down for one. Returns how many it appended, fewer
    /// only at the end of a sound that does not loop, or of one that cannot be read on.
    pub fn read(&mut self, out: &mut Vec<f32>, frames: usize, channels: usize) -> usize {
        let mut written = 0;
        while written < frames {
            if self.at + self.channels > self.packet.len() {
                if !self.next_packet() {
                    break;
                }
                continue;
            }
            let frame = &self.packet[self.at..self.at + self.channels];
            self.at += self.channels;
            let left = f32::from(frame[0]);
            let right = f32::from(frame[self.channels.min(2) - 1]);
            if channels == 1 {
                out.push((left + right) * 0.5);
            } else {
                out.push(left);
                out.push(right);
            }
            written += 1;
        }
        written
    }

    /// Decode the next packet, trimmed to where the stream says its audio ends, or begin again for a loop. Returns
    /// whether there is more.
    fn next_packet(&mut self) -> bool {
        let packet = guard(|| self.reader.read_dec_packet_itl()).unwrap_or_else(|error| {
            log::warn!("Ambience: a sound cannot be read on: {error}");
            self.failed = true;
            None
        });
        match packet {
            Some(mut packet) => {
                let frames = (packet.len() / self.channels) as u64;
                // Vorbis pads its last packet, and the last page's granule position says where the audio ends, so
                // that a loop joins without a gap.
                if let Some(end) = self.reader.get_last_absgp()
                    && self.decoded + frames > end
                {
                    packet.truncate(end.saturating_sub(self.decoded) as usize * self.channels);
                }
                self.decoded += (packet.len() / self.channels) as u64;
                self.heard |= !packet.is_empty();
                self.packet = packet;
                self.at = 0;
                true
            }
            None if self.looping && self.heard && !self.failed => {
                self.reader = match open(&self.bytes) {
                    Ok(reader) => reader,
                    Err(_) => unreachable!("the same bytes opened as the sound began"),
                };
                self.decoded = 0;
                self.heard = false;
                true
            }
            None => false,
        }
    }
}
