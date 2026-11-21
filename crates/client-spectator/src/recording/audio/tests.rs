// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use std::sync::{Mutex, mpsc::sync_channel};
use tomori_client::playback::{MixerOptions, Player, PlayerControl};
use tomori_protocol::{AudioFrame, Gate};

fn millis(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

fn block(at: Duration, rate: u32, layout: Layout, samples: Vec<i16>) -> Block {
    Block {
        at,
        rate,
        layout,
        samples,
    }
}

/// A block of `frames` stereo frames at 48 kHz, each of `value` in both channels, ending at `at`.
fn stereo(at: Duration, frames: usize, value: i16) -> Block {
    block(at, SAMPLE_RATE, Layout::Stereo, vec![value; frames * 2])
}

/// The little-endian 16-bit samples in `bytes`.
fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
        .collect()
}

#[test]
fn the_tap_copies_each_block_with_the_time_it_was_rendered_at() {
    let start = Instant::now();
    let (tap, blocks) = AudioTap::new(Origin::at(start));
    let mut tapped = tap.tap();
    tapped(44_100, Layout::Mono, &[1, 2, 3]);
    let block = blocks.try_recv().unwrap();
    assert_eq!(block.rate, 44_100);
    assert_eq!(block.layout, Layout::Mono);
    assert_eq!(block.samples, [1, 2, 3]);
    assert!(block.at <= start.elapsed());
    // Closed, it copies nothing more, and the blocks it dropped are counted when the writer is far behind.
    assert_eq!(tap.lost(), 0);
    for _ in 0..QUEUED {
        tapped(SAMPLE_RATE, Layout::Stereo, &[0, 0]);
    }
    assert_eq!(tap.lost(), 0);
    tapped(SAMPLE_RATE, Layout::Stereo, &[0, 0]);
    assert_eq!(tap.lost(), 1);
    tap.close();
    tapped(SAMPLE_RATE, Layout::Stereo, &[0, 0]);
    assert_eq!(tap.lost(), 1);
    drop(blocks);
    let (open, blocks) = AudioTap::new(Origin::at(start));
    drop(blocks);
    open.tap()(SAMPLE_RATE, Layout::Stereo, &[0, 0]);
    assert_eq!(open.lost(), 1);
}

#[test]
fn silence_fills_the_time_before_the_first_block() {
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    // 480 frames ending at one second begin 10 ms before it.
    aligner.block(&stereo(millis(1_000), 480, 100), &mut bytes);
    let written = samples(&bytes);
    assert_eq!(written.len(), (47_520 + 480) * 2);
    assert!(written[..47_520 * 2].iter().all(|&sample| sample == 0));
    assert!(written[47_520 * 2..].iter().all(|&sample| sample == 100));
    assert_eq!(aligner.written(), 48_000);
}

#[test]
fn blocks_that_arrive_when_expected_are_written_as_they_are() {
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    aligner.block(&stereo(millis(10), 480, 1), &mut bytes);
    let padded = aligner.padded;
    // A device's callbacks vary by a few milliseconds without the audio being moved.
    for (index, jitter) in [3_i64, -4, 2, 0, -3, 5].into_iter().enumerate() {
        let at = (20 + 10 * index as i64 + jitter) as u64;
        aligner.block(&stereo(millis(at), 480, 1), &mut bytes);
    }
    assert_eq!(aligner.padded, padded);
    assert_eq!(aligner.trimmed, 0);
    assert_eq!(aligner.written(), 7 * 480);
    assert!(samples(&bytes).iter().all(|&sample| sample == 1));
}

#[test]
fn a_block_that_comes_late_is_preceded_by_silence_and_one_that_comes_early_is_trimmed() {
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    aligner.block(&stereo(millis(10), 480, 1), &mut bytes);
    // The device stopped for 90 ms: the next block, 10 ms long, ends at 110 ms.
    let before = aligner.padded;
    aligner.block(&stereo(millis(110), 480, 2), &mut bytes);
    assert_eq!(aligner.padded - before, 4_320);
    assert_eq!(aligner.written(), 5_280);

    // 100 ms of audio is written. A block of 100 ms that ends at 150 ms begins at 50 ms, further back than the tolerance: its first 50 ms are left out.
    let mut aligner = Aligner::default();
    aligner.block(&stereo(millis(100), 4_800, 1), &mut Vec::new());
    assert_eq!(aligner.written(), 4_800);
    let mut bytes = Vec::new();
    aligner.block(&stereo(millis(150), 4_800, 9), &mut bytes);
    assert_eq!(aligner.trimmed, 2_400);
    assert_eq!(aligner.written(), 7_200);
    assert_eq!(bytes.len(), 2_400 * 4);
    // One entirely in the past is left out whole.
    bytes.clear();
    aligner.block(&stereo(millis(100), 2_400, 5), &mut bytes);
    assert_eq!(aligner.trimmed, 2_400 + 2_400);
    assert!(bytes.is_empty());
    assert_eq!(aligner.written(), 7_200);
}

#[test]
fn mono_is_played_in_both_channels() {
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    aligner.block(
        &block(millis(10), SAMPLE_RATE, Layout::Mono, vec![5; 480]),
        &mut bytes,
    );
    let written = samples(&bytes);
    assert_eq!(written.len(), 960);
    assert!(written.iter().all(|&sample| sample == 5));
}

#[test]
fn a_device_that_is_not_at_48_khz_is_resampled_and_a_changed_rate_starts_a_new_resampler() {
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    // A 440 Hz tone at 44.1 kHz, in blocks of 10 ms.
    let tone = |from: usize| -> Vec<i16> {
        (from..from + 441)
            .flat_map(|index| {
                let sample = (f64::sin(index as f64 * 440.0 * std::f64::consts::TAU / 44_100.0)
                    * 10_000.0) as i16;
                [sample, sample]
            })
            .collect()
    };
    for index in 0..50 {
        let at = millis(10 * (index as u64 + 1));
        aligner.block(
            &block(at, 44_100, Layout::Stereo, tone(index * 441)),
            &mut bytes,
        );
    }
    // Half a second at 44.1 kHz is half a second at 48 kHz, to within the frames the resampler holds back.
    assert!(
        (23_990..=24_000).contains(&aligner.written()),
        "{}",
        aligner.written()
    );
    assert_eq!(aligner.padded, 0);
    let left: Vec<i16> = samples(&bytes)
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| pair[0])
        .collect();
    // The tone's frequency is kept: 440 Hz has 220 rising zero crossings in half a second.
    let rising = left
        .windows(2)
        .filter(|pair| pair[0] < 0 && pair[1] >= 0)
        .count();
    assert!((218..=222).contains(&rising), "{rising}");
    assert!(left.iter().map(|&sample| sample.abs()).max().unwrap() > 9_900);
    // Back at 48 kHz the blocks are written as they are.
    let before = aligner.written();
    aligner.block(&stereo(millis(510), 480, 3), &mut Vec::new());
    assert_eq!(aligner.written(), before + 480);
    assert!(aligner.resampler.is_none());
    // A different rate is resampled afresh.
    aligner.block(
        &block(millis(520), 24_000, Layout::Stereo, vec![0; 480]),
        &mut Vec::new(),
    );
    assert_eq!(aligner.resampler.as_ref().unwrap().0, 24_000);
    aligner.block(
        &block(millis(530), 0, Layout::Stereo, vec![0; 480]),
        &mut Vec::new(),
    );
    assert!(aligner.resampler.is_none());
}

#[test]
fn resampling_in_blocks_gives_what_resampling_all_at_once_does() {
    let input: Vec<i16> = (0..2_000).flat_map(|n| [n as i16, -(n as i16)]).collect();
    let mut whole = Vec::new();
    Resampler::new(44_100, 48_000).push(&input, &mut whole);
    let mut resampler = Resampler::new(44_100, 48_000);
    let mut pieces = Vec::new();
    for piece in input.chunks(2 * 37) {
        resampler.push(piece, &mut pieces);
    }
    assert_eq!(pieces, whole);
    assert!(whole.len() > 2_000 * 2);
    // Down, too.
    let mut down = Vec::new();
    Resampler::new(96_000, 48_000).push(&input, &mut down);
    assert!((down.len() as i64 - 2_000).abs() <= 4, "{}", down.len());
}

#[test]
fn nothing_arriving_leaves_silence_only_once_the_audio_is_well_behind() {
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    aligner.idle(millis(99), &mut bytes);
    assert!(bytes.is_empty());
    aligner.idle(millis(101), &mut bytes);
    assert_eq!(aligner.written(), 4_848);
    assert_eq!(bytes.len(), 4_848 * 4);
    assert_eq!(aligner.padded, 4_848);
    aligner.idle(millis(150), &mut bytes);
    assert_eq!(aligner.written(), 4_848);
}

#[test]
fn the_end_is_silence_up_to_the_length_of_the_video_and_never_less() {
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    aligner.block(&stereo(millis(500), 24_000, 1), &mut bytes);
    bytes.clear();
    aligner.finish(48_000, &mut bytes);
    assert_eq!(aligner.written(), 48_000);
    assert_eq!(bytes.len(), 24_000 * 4);
    bytes.clear();
    aligner.finish(40_000, &mut bytes);
    assert!(bytes.is_empty());
    assert_eq!(aligner.written(), 48_000);
}

/// A pipe that keeps what is written to it.
#[derive(Clone, Default)]
struct Kept(Arc<Mutex<Vec<u8>>>);

impl Write for Kept {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Broken;

impl Write for Broken {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_writer_puts_blocks_on_the_clock_and_ends_with_the_length_of_the_video() {
    let start = Instant::now();
    let (tap, blocks) = AudioTap::new(Origin::at(start));
    let (kept, ending, stop) = (
        Kept::default(),
        Arc::new(Ending::default()),
        Stop::default(),
    );
    let clock = FrameClock::new(30);
    let writer = write(
        kept.clone(),
        blocks,
        Origin::at(start),
        clock,
        ending.clone(),
        stop.clone(),
    )
    .unwrap();
    let mut tapped = tap.tap();
    tapped(SAMPLE_RATE, Layout::Stereo, &vec![7; 960]);
    thread::sleep(millis(50));
    tapped(SAMPLE_RATE, Layout::Stereo, &vec![7; 960]);
    // Half a second: 15 frames at 30 a second, which is 24,000 frames of sound.
    ending.end(millis(500));
    let totals = writer.join().unwrap();
    assert_eq!(totals.frames, 24_000);
    assert_eq!(totals.error, None);
    assert_eq!(kept.0.lock().unwrap().len(), 24_000 * 4);
    assert!(totals.padded > 0);
    assert!(!stop.requested());
}

#[test]
fn what_was_rendered_before_the_end_is_written_before_it() {
    let start = Instant::now();
    let (tap, blocks) = AudioTap::new(Origin::at(start));
    // Five blocks wait when the recording ends, a tenth of a second after it began.
    for index in 1..=5 {
        tap.sender.send(stereo(millis(10 * index), 480, 3)).unwrap();
    }
    let (kept, ending) = (Kept::default(), Arc::new(Ending::default()));
    ending.end(millis(100));
    let writer = write(
        kept.clone(),
        blocks,
        Origin::at(start),
        FrameClock::new(10),
        ending,
        Stop::default(),
    )
    .unwrap();
    let totals = writer.join().unwrap();
    assert_eq!(totals.frames, 4_800);
    let written = samples(&kept.0.lock().unwrap());
    assert_eq!(written.len(), 4_800 * 2);
    assert!(written[..2_400 * 2].iter().all(|&sample| sample == 3));
    assert!(written[2_400 * 2..].iter().all(|&sample| sample == 0));
}

#[test]
fn a_writer_that_cannot_be_started_is_reported() {
    let (_tap, blocks) = AudioTap::new(Origin::at(Instant::now()));
    system::refuse("tomori-record-audio");
    let error = write(
        Kept::default(),
        blocks,
        Origin::at(Instant::now()),
        FrameClock::new(30),
        Arc::default(),
        Stop::default(),
    )
    .map(drop)
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the system refused to start tomori-record-audio"
    );
}

#[test]
fn a_sound_that_cannot_be_written_stops_the_recording() {
    let start = Instant::now();
    let (tap, blocks) = AudioTap::new(Origin::at(start));
    let (ending, stop) = (Arc::new(Ending::default()), Stop::default());
    let writer = write(
        Broken,
        blocks,
        Origin::at(start),
        FrameClock::new(30),
        ending,
        stop.clone(),
    )
    .unwrap();
    tap.tap()(SAMPLE_RATE, Layout::Stereo, &[1; 960]);
    let totals = writer.join().unwrap();
    assert_eq!(
        totals.error.as_deref(),
        Some("cannot write the sound to ffmpeg: broken pipe")
    );
    assert_eq!(stop.reason().as_deref(), Some("the encoder stopped"));
}

#[test]
fn the_writer_waits_for_blocks_if_the_tap_has_gone_and_fills_the_time() {
    let start = Instant::now();
    let (tap, blocks) = AudioTap::new(Origin::at(start));
    drop(tap);
    let (kept, ending) = (Kept::default(), Arc::new(Ending::default()));
    let writer = write(
        kept.clone(),
        blocks,
        Origin::at(start),
        FrameClock::new(10),
        ending.clone(),
        Stop::default(),
    )
    .unwrap();
    thread::sleep(millis(150));
    ending.end(millis(200));
    let totals = writer.join().unwrap();
    assert_eq!(totals.frames, 9_600);
    assert!(totals.padded >= 9_600, "{}", totals.padded);
}

/// A speaker that keeps the player it is asked to play, for the test to render as its callback would, or refuses to open.
#[derive(Clone, Default)]
struct Speaker {
    player: Arc<Mutex<Option<Shared>>>,
    refuses: bool,
    focus: bool,
}

impl Playback for Speaker {
    fn open(&self, player: Shared, _failed: Failed, _route: Route) -> Result<Output, String> {
        if self.refuses {
            return Err("No speaker is available.".into());
        }
        *self.player.lock().unwrap() = Some(player);
        Ok(Output::new(()))
    }

    fn reopens_on_focus(&self) -> bool {
        self.focus
    }
}

fn frame(source: u32, sequence: u32, value: i16) -> AudioFrame {
    AudioFrame {
        gate: Gate::Live,
        source,
        sequence,
        timestamp: 0,
        sample_rate: SAMPLE_RATE,
        samples: vec![value; 960],
    }
}

fn player() -> (
    mpsc::SyncSender<AudioFrame>,
    Shared,
    PlayerControl,
    mpsc::SyncSender<AudioFrame>,
) {
    let (sender, audio) = sync_channel(64);
    let (measure, measured) = sync_channel(64);
    let mut options = MixerOptions::default();
    options.sample_rate = SAMPLE_RATE;
    options.jitter.target_latency = millis(20);
    options.jitter.latency_limit = millis(400);
    let (mut player, control) = Player::new(audio, options);
    player.measure(measured);
    (sender, Arc::new(Mutex::new(player)), control, measure)
}

fn fail() -> Failed {
    Box::new(|_| {})
}

#[test]
fn the_speaker_plays_exactly_what_it_would_and_the_recording_is_given_a_copy() {
    let (tap, blocks) = AudioTap::new(Origin::at(Instant::now()));
    let speaker = Speaker::default();
    let tee = Tee::new(Some(Box::new(speaker.clone())), tap);
    let (sender, shared, control, _measure) = player();
    let (other_sender, other, other_control, _other_measure) = player();
    // The speaker is opened as the connection opens it, with the player.
    let _output = tee.open(shared.clone(), fail(), Route::default()).unwrap();
    assert!(Arc::ptr_eq(
        speaker.player.lock().unwrap().as_ref().unwrap(),
        &shared
    ));
    for control in [&control, &other_control] {
        control.set_agent_muted(false);
    }
    for sequence in 0..4 {
        for sender in [&sender, &other_sender] {
            sender.send(frame(1, sequence, 100)).unwrap();
            sender
                .send(frame(tomori_protocol::AGENT_SOURCE, sequence, 20))
                .unwrap();
        }
    }
    let (mut played, mut untapped) = (vec![0; 960], vec![0; 960]);
    let mut copies = Vec::new();
    for _ in 0..6 {
        // What the device's callback does, at the device's rate and layout.
        shared
            .lock()
            .unwrap()
            .render(SAMPLE_RATE, Layout::Stereo, &mut played);
        other
            .lock()
            .unwrap()
            .render(SAMPLE_RATE, Layout::Stereo, &mut untapped);
        // The speaker plays what an untapped player would.
        assert_eq!(played, untapped);
        let copy = blocks.try_recv().unwrap();
        assert_eq!((copy.rate, copy.layout), (SAMPLE_RATE, Layout::Stereo));
        assert_eq!(copy.samples, played);
        copies.push(copy.samples);
    }
    assert!(copies.iter().flatten().any(|&sample| sample != 0));
    assert!(!tee.reopens_on_focus());
    let tee = Tee::new(
        Some(Box::new(Speaker {
            focus: true,
            ..Speaker::default()
        })),
        AudioTap::new(Origin::at(Instant::now())).0,
    );
    assert!(tee.reopens_on_focus());
}

#[test]
fn the_recording_hears_the_voices_played_and_not_those_muted_or_only_measured() {
    let (tap, blocks) = AudioTap::new(Origin::at(Instant::now()));
    let speaker = Speaker::default();
    let tee = Tee::new(Some(Box::new(speaker)), tap);
    let (sender, shared, control, measure) = player();
    let _output = tee.open(shared.clone(), fail(), Route::default()).unwrap();
    // The agent muted, as the spectator mutes it with `--voices none`, and a voice only measured, as `--duck-under everyone` measures one it does not play: the recording has the voice played alone.
    control.set_agent_muted(true);
    for sequence in 0..4 {
        sender.send(frame(2, sequence, 1_000)).unwrap();
        sender
            .send(frame(tomori_protocol::AGENT_SOURCE, sequence, 5_000))
            .unwrap();
        measure.send(frame(3, sequence, 20_000)).unwrap();
    }
    let mut buffer = vec![0; 960];
    let mut loudest = 0;
    for _ in 0..6 {
        shared
            .lock()
            .unwrap()
            .render(SAMPLE_RATE, Layout::Stereo, &mut buffer);
        let copy = blocks.try_recv().unwrap();
        assert_eq!(copy.samples, buffer);
        loudest = copy
            .samples
            .iter()
            .map(|sample| sample.abs())
            .fold(loudest, i16::max);
    }
    assert!(loudest > 0 && loudest <= 1_000, "{loudest}");
}

#[test]
fn without_a_speaker_the_recording_renders_the_player_itself_at_48_khz() {
    let (tap, blocks) = AudioTap::new(Origin::at(Instant::now()));
    let tee = Tee::new(None, tap);
    let (_sender, shared, _control, _measure) = player();
    let output = tee.open(shared, fail(), Route::default()).unwrap();
    let block = blocks.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!((block.rate, block.layout), (SAMPLE_RATE, Layout::Stereo));
    assert_eq!(block.samples.len(), 480 * 2);
    // Dropping the output ends the thread that renders it: nothing more arrives.
    drop(output);
    while blocks.try_recv().is_ok() {}
    thread::sleep(millis(50));
    assert!(blocks.try_recv().is_err());
}

#[test]
fn a_speaker_that_cannot_be_opened_leaves_the_recording_its_sound() {
    log::set_max_level(log::LevelFilter::Trace);
    let (tap, blocks) = AudioTap::new(Origin::at(Instant::now()));
    let tee = Tee::new(
        Some(Box::new(Speaker {
            refuses: true,
            ..Speaker::default()
        })),
        tap,
    );
    let (_sender, shared, _control, _measure) = player();
    let _output = tee.open(shared, fail(), Route::default()).unwrap();
    assert!(blocks.recv_timeout(Duration::from_secs(10)).is_ok());
}

#[test]
fn without_the_thread_to_render_it_the_sound_cannot_be_recorded_and_the_connection_is_told() {
    let (tap, _blocks) = AudioTap::new(Origin::at(Instant::now()));
    let tee = Tee::new(None, tap);
    let (_sender, shared, _control, _measure) = player();
    system::refuse("tomori-record-player");
    let error = tee
        .open(shared, fail(), Route::default())
        .err()
        .expect("refused");
    assert_eq!(
        error,
        "Cannot start the thread that renders the recording's sound: the system refused to start tomori-record-player"
    );
}

#[test]
fn the_sound_stays_within_the_tolerance_of_the_clock_however_the_callbacks_vary() {
    // A device whose callbacks come every 10 ms of its own clock, which runs 100 parts in a million fast against the recording's, with each callback up to 15 ms early or late, and now and then a stall of up to 200 ms after which they come in a burst. The block's time is when it was rendered, and the sound is never further from the clock than the tolerance, after any block.
    let mut aligner = Aligner::default();
    let mut bytes = Vec::new();
    let mut seed = 12_345_u64;
    let mut random = move |below: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % below
    };
    let (mut worst, mut previous) = (0_i64, Duration::ZERO);
    let mut stalled_until = Duration::ZERO;
    for index in 1..=6_000_u64 {
        let ideal = Duration::from_nanos(index * 9_999_000);
        let jitter = random(31) as i64 - 15;
        let mut at = if jitter < 0 {
            ideal.saturating_sub(millis(-jitter as u64))
        } else {
            ideal + millis(jitter as u64)
        };
        if random(200) == 0 {
            stalled_until = ideal + millis(random(200));
        }
        // After a stall they come together, at the time it ended; and they never go back in time.
        at = at.max(stalled_until).max(previous);
        previous = at;
        aligner.block(&stereo(at, 480, 1), &mut bytes);
        bytes.clear();
        let off = aligner.written() as i64 - frames_in(at);
        worst = worst.max(off.abs());
        assert!(
            off.abs() <= TOLERANCE,
            "block {index} at {at:?} is {off} frames off"
        );
    }
    // It is corrected, at a cost of silence or of sound left out, and not left to drift.
    assert!(aligner.padded + aligner.trimmed > 0);
    assert!(worst > 0);
}
