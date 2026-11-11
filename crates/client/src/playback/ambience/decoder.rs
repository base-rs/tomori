// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Decoding a slot's Ogg Vorbis file a packet at a time, on the ambience's own thread, from its bytes in memory, so
//! that no sound is ever held decoded whole: mono or stereo out, whichever the voice wants, over and over for a loop.
use std::io::Cursor;
use std::sync::Arc;

use lewton::inside_ogg::OggStreamReader;

type Reader = OggStreamReader<Cursor<Arc<[u8]>>>;

fn open(bytes: &Arc<[u8]>) -> Result<Reader, String> {
    OggStreamReader::new(Cursor::new(bytes.clone())).map_err(|error| error.to_string())
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
}

impl Decoder {
    /// Decode `bytes`, once, or over and over if `looping`. Refused if they are not Ogg Vorbis.
    pub fn new(bytes: Arc<[u8]>, looping: bool) -> Result<Self, String> {
        let reader = open(&bytes)?;
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
        })
    }

    /// Its samples a second.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Append up to `frames` frames to `out`, in `channels`, one or two, in `i16` units: a mono file heard in both of
    /// two, and a stereo one, or the first two channels of more, mixed down for one. Returns how many it appended, fewer
    /// only at the end of a sound that does not loop, or of one that cannot be read on.
    pub fn read(&mut self, out: &mut Vec<f32>, frames: usize, channels: usize) -> usize {
        let mut written = 0;
        while written < frames {
            if self.at >= self.packet.len() {
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
        let packet = self.reader.read_dec_packet_itl().unwrap_or_else(|error| {
            log::warn!("Ambience: a sound cannot be read on: {error}");
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
            None if self.looping && self.heard => {
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
