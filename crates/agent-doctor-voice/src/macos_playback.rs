//! Play a reply through the same audio engine as the microphone.
//!
//! Voice processing removes the audio being played from the mic. The barge
//! gate then decides whether the person has started talking.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use block2::{RcBlock, StackBlock};
use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2::sel;

use super::activity::{EchoCleanedActivity, SpeechActivity};
use super::barge::{self, BargeAction};
use super::macos::{buffer_rms, guard_audio_call, install_input_tap, on_main, require_class};
use super::types::{SpeechError, SpeechErrorCode};
use objc2_foundation::NSString;

pub(super) const PLAYBACK_UNAVAILABLE: &str = "barge playback unavailable";

/// Once this Mac refuses echo-cancelled playback, later replies skip the slow attempt.
static ECHO_PATH_BROKEN: AtomicBool = AtomicBool::new(false);

struct Piece(usize);
unsafe impl Send for Piece {}

pub(super) fn speak_with_barge(
    text: &str,
    language: Option<&str>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    if ECHO_PATH_BROKEN.load(Ordering::Relaxed) {
        return Err(unavailable());
    }
    let (tx, rx) = mpsc::channel::<Option<Piece>>();
    let synth = start_writing(text, language, tx)?;
    let first = match rx.recv_timeout(Duration::from_millis(1200)) {
        Ok(Some(piece)) => piece,
        Ok(None) => {
            stop_synth(synth);
            return Err(unavailable());
        }
        Err(_) => {
            stop_synth(synth);
            ECHO_PATH_BROKEN.store(true, Ordering::Relaxed);
            eprintln!("[voice] barge fallback: speech audio was slow");
            return Err(unavailable());
        }
    };
    if should_cancel() {
        release_piece(first);
        stop_synth(synth);
        return Err(cancelled());
    }

    let interrupt = Arc::new(AtomicBool::new(false));
    let held_ms = Arc::new(AtomicU64::new(0));
    let armed = Arc::new(AtomicBool::new(false));
    // One try. Repeating a failed audio graph is what made speech wait several seconds.
    let playback = match start_engine(first.0, interrupt.clone(), held_ms.clone(), armed.clone()) {
        Ok(started) => started,
        Err(err) => {
            release_bits(first.0);
            stop_synth(synth);
            ECHO_PATH_BROKEN.store(true, Ordering::Relaxed);
            eprintln!("[voice] barge fallback: {}", err.detail);
            return Err(unavailable());
        }
    };
    eprintln!("[voice] barge playback");

    let started = Instant::now();
    let mut planned = playback.planned;
    let result = play_until_done(
        &rx,
        &playback,
        &interrupt,
        &armed,
        started,
        &mut planned,
        should_cancel,
    );
    stop_engine(&playback);
    stop_synth(synth);
    result
}

struct Playback {
    engine: usize,
    player: usize,
    input: usize,
    format: usize,
    planned: Duration,
}

fn play_until_done(
    rx: &Receiver<Option<Piece>>,
    playback: &Playback,
    interrupt: &AtomicBool,
    armed: &AtomicBool,
    started: Instant,
    planned: &mut Duration,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    let mut synth_done = false;
    loop {
        if started.elapsed() >= Duration::from_millis(barge::ARM_AFTER_MS) {
            armed.store(true, Ordering::Relaxed);
        }
        if should_cancel() {
            return Err(cancelled());
        }
        if interrupt.load(Ordering::Relaxed) {
            eprintln!("[voice] barge interrupt");
            return Ok(());
        }
        if synth_done && started.elapsed() >= *planned + Duration::from_millis(150) {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(180) {
            return Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(Some(piece)) => {
                let extra = schedule_piece(playback.player, playback.format, piece);
                *planned += extra;
            }
            Ok(None) => synth_done = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => synth_done = true,
        }
    }
}

fn start_writing(
    text: &str,
    language: Option<&str>,
    tx: mpsc::Sender<Option<Piece>>,
) -> Result<usize, SpeechError> {
    let text = text.to_string();
    let language = language.map(str::to_string);
    on_main(move || unsafe {
        let synth_class = require_class(c"AVSpeechSynthesizer")?;
        let synth: *mut AnyObject = msg_send![synth_class, new];
        if synth.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Unavailable,
                "speech synthesizer unavailable",
            ));
        }
        let utterance_class = require_class(c"AVSpeechUtterance")?;
        let spoken = NSString::from_str(&text);
        let utterance: *mut AnyObject =
            msg_send![utterance_class, speechUtteranceWithString: &*spoken];
        if utterance.is_null() {
            let _: () = msg_send![synth, release];
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "failed to create speech utterance",
            ));
        }
        let can_write: bool =
            msg_send![synth, respondsToSelector: sel!(writeUtterance:toBufferCallback:)];
        if !can_write {
            let _: () = msg_send![synth, release];
            return Err(unavailable());
        }
        if let Some(lang) = language.as_deref() {
            let voice_class = require_class(c"AVSpeechSynthesisVoice")?;
            let lang_ns = NSString::from_str(lang);
            let voice: *mut AnyObject = msg_send![voice_class, voiceWithLanguage: &*lang_ns];
            if !voice.is_null() {
                let _: () = msg_send![utterance, setVoice: voice];
            }
        }
        let block = StackBlock::new(move |buffer: *mut AnyObject| {
            if buffer.is_null() {
                let _ = tx.send(None);
                return;
            }
            let frames: usize = msg_send![buffer, frameLength];
            if frames == 0 {
                let _ = tx.send(None);
                return;
            }
            let _: *mut AnyObject = msg_send![buffer, retain];
            if tx.send(Some(Piece(buffer as usize))).is_err() {
                let _: () = msg_send![buffer, release];
            }
        });
        let block: RcBlock<dyn Fn(*mut AnyObject)> = block.copy();
        let _: () = msg_send![synth, writeUtterance: utterance, toBufferCallback: &*block];
        // The synthesizer retains the callback. Keep our copy until writing ends
        // by leaking it into the synth's lifetime: store it on a side box that
        // lives as long as `synth` by attaching it as an associated... we just
        // forget this one copy; stop_synth ends the callbacks.
        std::mem::forget(block);
        Ok(synth as usize)
    })
}

fn start_engine(
    first_bits: usize,
    interrupt: Arc<AtomicBool>,
    held_ms: Arc<AtomicU64>,
    armed: Arc<AtomicBool>,
) -> Result<Playback, SpeechError> {
    on_main(move || unsafe {
        let engine_class = require_class(c"AVAudioEngine")?;
        let engine: *mut AnyObject = msg_send![engine_class, new];
        if engine.is_null() {
            return Err(unavailable());
        }
        let input: *mut AnyObject = msg_send![engine, inputNode];
        let output: *mut AnyObject = msg_send![engine, outputNode];
        if input.is_null() || output.is_null() {
            let _: () = msg_send![engine, release];
            return Err(unavailable());
        }
        let mut vp_error: *mut AnyObject = std::ptr::null_mut();
        let vp_ok: bool = msg_send![input, setVoiceProcessingEnabled: true, error: &mut vp_error];
        if !vp_ok {
            let detail = error_text(vp_error);
            let _: () = msg_send![engine, release];
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                format!("voice processing: {detail}"),
            ));
        }
        let mut vp_out_error: *mut AnyObject = std::ptr::null_mut();
        let vp_out_ok: bool =
            msg_send![output, setVoiceProcessingEnabled: true, error: &mut vp_out_error];
        if !vp_out_ok {
            eprintln!(
                "[voice] barge output processing: {}",
                error_text(vp_out_error)
            );
        }

        let player_class = require_class(c"AVAudioPlayerNode")?;
        let player: *mut AnyObject = msg_send![player_class, new];
        if player.is_null() {
            let _: () = msg_send![engine, release];
            return Err(unavailable());
        }
        let _: () = msg_send![engine, attachNode: player];
        let buffer = first_bits as *mut AnyObject;
        let mut hw_format: *mut AnyObject = msg_send![output, inputFormatForBus: 0usize];
        let out_rate: f64 = if hw_format.is_null() {
            0.0
        } else {
            msg_send![hw_format, sampleRate]
        };
        if hw_format.is_null() || out_rate < 1.0 {
            hw_format = msg_send![input, outputFormatForBus: 0usize];
        }
        if hw_format.is_null() {
            let _: () = msg_send![player, release];
            let _: () = msg_send![engine, release];
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "microphone format unavailable",
            ));
        }
        let _: *mut AnyObject = msg_send![hw_format, retain];
        let mixer: *mut AnyObject = msg_send![engine, mainMixerNode];
        // Voice processing only starts when every node uses the microphone rate.
        // Speech buffers are a different rate, so they are converted before play.
        if let Err(err) = guard_audio_call(AssertUnwindSafe(|| {
            let _: () = msg_send![engine, connect: player, to: mixer, format: hw_format];
        })) {
            let _: () = msg_send![hw_format, release];
            let _: () = msg_send![player, release];
            let _: () = msg_send![engine, release];
            return Err(err);
        }

        let mut engine_error: *mut AnyObject = std::ptr::null_mut();
        let started: bool = msg_send![engine, startAndReturnError: &mut engine_error];
        if !started {
            let detail = error_text(engine_error);
            let _: () = msg_send![hw_format, release];
            let _: () = msg_send![player, release];
            let _: () = msg_send![engine, release];
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                format!("audio engine: {detail}"),
            ));
        }

        let interrupt_for_tap = interrupt.clone();
        let held_for_tap = held_ms.clone();
        let armed_for_tap = armed.clone();
        let tap = StackBlock::new(move |buffer: *mut AnyObject, _when: *mut AnyObject| {
            if buffer.is_null() {
                return;
            }
            let frames: usize = msg_send![buffer, frameLength];
            let format: *mut AnyObject = msg_send![buffer, format];
            let rate: f64 = if format.is_null() {
                0.0
            } else {
                msg_send![format, sampleRate]
            };
            let frame_ms = if rate > 0.0 {
                ((frames as f64 / rate) * 1000.0) as u64
            } else {
                20
            };
            let hearing = EchoCleanedActivity::default().hearing_speech(buffer_rms(buffer));
            let held = held_for_tap.load(Ordering::Relaxed);
            let (next, action) = barge::advance(
                held,
                armed_for_tap.load(Ordering::Relaxed),
                hearing,
                frame_ms,
            );
            held_for_tap.store(next, Ordering::Relaxed);
            if action == BargeAction::Interrupt {
                interrupt_for_tap.store(true, Ordering::Relaxed);
            }
        });
        let tap = tap.copy();
        if let Err(err) = install_input_tap(input, &tap) {
            let _: () = msg_send![engine, stop];
            let _: () = msg_send![hw_format, release];
            let _: () = msg_send![player, release];
            let _: () = msg_send![engine, release];
            return Err(err);
        }
        std::mem::forget(tap);

        let planned = schedule_buffer(player, hw_format, buffer);
        let _: () = msg_send![player, play];
        Ok(Playback {
            engine: engine as usize,
            player: player as usize,
            input: input as usize,
            format: hw_format as usize,
            planned,
        })
    })
}

fn schedule_piece(player_bits: usize, format_bits: usize, piece: Piece) -> Duration {
    on_main(move || unsafe {
        schedule_buffer(
            player_bits as *mut AnyObject,
            format_bits as *mut AnyObject,
            piece.0 as *mut AnyObject,
        )
    })
}

unsafe fn schedule_buffer(
    player: *mut AnyObject,
    target_format: *mut AnyObject,
    buffer: *mut AnyObject,
) -> Duration {
    let playable = match convert_buffer(buffer, target_format) {
        Some(playable) => playable,
        None => {
            let _: () = msg_send![buffer, release];
            return Duration::ZERO;
        }
    };
    let _: () = msg_send![buffer, release];
    let frames: usize = msg_send![playable, frameLength];
    let rate: f64 = msg_send![target_format, sampleRate];
    if guard_audio_call(AssertUnwindSafe(|| {
        let _: () = unsafe { msg_send![player, scheduleBuffer: playable] };
    }))
    .is_err()
    {
        let _: () = msg_send![playable, release];
        return Duration::ZERO;
    }
    let _: () = msg_send![playable, release];
    if rate > 0.0 {
        Duration::from_secs_f64(frames as f64 / rate)
    } else {
        Duration::from_millis(20)
    }
}

unsafe fn convert_buffer(
    src: *mut AnyObject,
    target_format: *mut AnyObject,
) -> Option<*mut AnyObject> {
    let (samples, in_rate) = read_mono(src)?;
    let out_rate: f64 = msg_send![target_format, sampleRate];
    if out_rate < 1.0 || samples.is_empty() {
        return None;
    }
    let converted = resample(&samples, in_rate, out_rate);
    write_mono(target_format, &converted)
}

unsafe fn read_mono(buffer: *mut AnyObject) -> Option<(Vec<f32>, f64)> {
    let frames: usize = msg_send![buffer, frameLength];
    if frames == 0 {
        return None;
    }
    let format: *mut AnyObject = msg_send![buffer, format];
    if format.is_null() {
        return None;
    }
    let rate: f64 = msg_send![format, sampleRate];
    let float_channels: *mut *mut f32 = msg_send![buffer, floatChannelData];
    if !float_channels.is_null() && !(*float_channels).is_null() {
        let samples = std::slice::from_raw_parts(*float_channels, frames);
        return Some((samples.to_vec(), rate));
    }
    let int_channels: *mut *mut i16 = msg_send![buffer, int16ChannelData];
    if !int_channels.is_null() && !(*int_channels).is_null() {
        let samples = std::slice::from_raw_parts(*int_channels, frames);
        return Some((
            samples
                .iter()
                .map(|sample| *sample as f32 / 32768.0)
                .collect(),
            rate,
        ));
    }
    None
}

unsafe fn write_mono(format: *mut AnyObject, samples: &[f32]) -> Option<*mut AnyObject> {
    if samples.is_empty() {
        return None;
    }
    let class = require_class(c"AVAudioPCMBuffer").ok()?;
    let buffer: *mut AnyObject = msg_send![class, alloc];
    let buffer: *mut AnyObject =
        msg_send![buffer, initWithPCMFormat: format, frameCapacity: samples.len() as u32];
    if buffer.is_null() {
        return None;
    }
    let _: () = msg_send![buffer, setFrameLength: samples.len() as u32];
    let channels: u32 = msg_send![format, channelCount];
    let common: usize = msg_send![format, commonFormat];
    if common == 1 {
        let data: *mut *mut f32 = msg_send![buffer, floatChannelData];
        if data.is_null() {
            let _: () = msg_send![buffer, release];
            return None;
        }
        for channel in 0..channels.max(1) as usize {
            let dst = *data.add(channel);
            if dst.is_null() {
                continue;
            }
            std::ptr::copy_nonoverlapping(samples.as_ptr(), dst, samples.len());
        }
        return Some(buffer);
    }
    if common == 3 {
        let data: *mut *mut i16 = msg_send![buffer, int16ChannelData];
        if data.is_null() {
            let _: () = msg_send![buffer, release];
            return None;
        }
        for channel in 0..channels.max(1) as usize {
            let dst = *data.add(channel);
            if dst.is_null() {
                continue;
            }
            for (index, sample) in samples.iter().enumerate() {
                *dst.add(index) = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            }
        }
        return Some(buffer);
    }
    let _: () = msg_send![buffer, release];
    None
}

fn resample(input: &[f32], in_rate: f64, out_rate: f64) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    if in_rate < 1.0 || out_rate < 1.0 || (in_rate - out_rate).abs() < 1.0 {
        return input.to_vec();
    }
    let out_len = ((input.len() as f64) * out_rate / in_rate).round().max(1.0) as usize;
    let mut output = Vec::with_capacity(out_len);
    let last = input.len() - 1;
    for index in 0..out_len {
        let position = index as f64 * in_rate / out_rate;
        let left = (position.floor() as usize).min(last);
        let right = (left + 1).min(last);
        let frac = (position - left as f64) as f32;
        output.push(input[left] + (input[right] - input[left]) * frac);
    }
    output
}

fn stop_engine(playback: &Playback) {
    let engine = playback.engine;
    let player = playback.player;
    let input = playback.input;
    let format = playback.format;
    on_main(move || unsafe {
        let player = player as *mut AnyObject;
        let engine = engine as *mut AnyObject;
        let input = input as *mut AnyObject;
        let format = format as *mut AnyObject;
        let _: () = msg_send![player, stop];
        let _: () = msg_send![engine, stop];
        let _: () = msg_send![input, removeTapOnBus: 0usize];
        let _: () = msg_send![player, release];
        let _: () = msg_send![engine, release];
        let _: () = msg_send![format, release];
    });
}

fn stop_synth(synth_bits: usize) {
    on_main(move || unsafe {
        let synth = synth_bits as *mut AnyObject;
        let _: () = msg_send![synth, stopSpeakingAtBoundary: 0isize];
        let _: () = msg_send![synth, release];
    });
}

fn release_piece(piece: Piece) {
    release_bits(piece.0);
}

fn release_bits(bits: usize) {
    on_main(move || unsafe {
        let buffer = bits as *mut AnyObject;
        let _: () = msg_send![buffer, release];
    });
}

unsafe fn error_text(err: *mut AnyObject) -> String {
    if err.is_null() {
        return "no system detail".to_string();
    }
    let desc: *mut AnyObject = msg_send![err, localizedDescription];
    if desc.is_null() {
        return "no system detail".to_string();
    }
    let ns_str = &*(desc as *const NSString);
    ns_str.to_string()
}

fn unavailable() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Failed, PLAYBACK_UNAVAILABLE)
}

fn cancelled() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Cancelled, "speech cancelled")
}

/// Mic monitor used when echo-cancelled playback cannot start.
/// The first moments measure speaker bleed. A louder voice stops playback.
pub(super) struct BleedWatch {
    engine: usize,
    input: usize,
    interrupt: Arc<AtomicBool>,
}

impl BleedWatch {
    pub(super) fn interrupted(&self) -> bool {
        self.interrupt.load(Ordering::Relaxed)
    }

    pub(super) fn stop(self) {
        let engine = self.engine;
        let input = self.input;
        on_main(move || unsafe {
            let engine = engine as *mut AnyObject;
            let input = input as *mut AnyObject;
            let _: () = msg_send![engine, stop];
            let _: () = msg_send![input, removeTapOnBus: 0usize];
            let _: () = msg_send![engine, release];
        });
    }
}

pub(super) fn start_bleed_watch() -> Result<BleedWatch, SpeechError> {
    let interrupt = Arc::new(AtomicBool::new(false));
    let interrupt_for_tap = interrupt.clone();
    let (engine, input) = on_main(move || unsafe {
        let engine_class = require_class(c"AVAudioEngine")?;
        let engine: *mut AnyObject = msg_send![engine_class, new];
        if engine.is_null() {
            return Err(unavailable());
        }
        let input: *mut AnyObject = msg_send![engine, inputNode];
        if input.is_null() {
            let _: () = msg_send![engine, release];
            return Err(unavailable());
        }
        let gate = Arc::new(Mutex::new((barge::BleedGate::new(), 0.0_f32, 0_u64)));
        let tap = StackBlock::new(move |buffer: *mut AnyObject, _when: *mut AnyObject| {
            if buffer.is_null() {
                return;
            }
            let frames: usize = msg_send![buffer, frameLength];
            let format: *mut AnyObject = msg_send![buffer, format];
            let rate: f64 = if format.is_null() {
                0.0
            } else {
                msg_send![format, sampleRate]
            };
            let frame_ms = if rate > 0.0 {
                ((frames as f64 / rate) * 1000.0) as u64
            } else {
                20
            };
            let rms = buffer_rms(buffer);
            let mut state = gate.lock().unwrap_or_else(|err| err.into_inner());
            let (gate, peak, window_ms) = &mut *state;
            let action = gate.push(rms, frame_ms.max(1));
            *peak = peak.max(rms);
            *window_ms += frame_ms.max(1);
            if *window_ms >= 1000 {
                eprintln!(
                    "[voice] barge watch: speaker {:.3}, loudest {:.3}, needs {:.3}",
                    gate.floor(),
                    *peak,
                    barge::bleed_line(gate.floor())
                );
                *peak = 0.0;
                *window_ms = 0;
            }
            if action == BargeAction::Interrupt {
                interrupt_for_tap.store(true, Ordering::Relaxed);
            }
        });
        let tap = tap.copy();
        if let Err(err) = install_input_tap(input, &tap) {
            eprintln!("[voice] barge watch failed: {}", err.detail);
            let _: () = msg_send![engine, release];
            return Err(unavailable());
        }
        std::mem::forget(tap);
        let mut engine_error: *mut AnyObject = std::ptr::null_mut();
        let started: bool = msg_send![engine, startAndReturnError: &mut engine_error];
        if !started {
            eprintln!("[voice] barge watch failed: {}", error_text(engine_error));
            let _: () = msg_send![input, removeTapOnBus: 0usize];
            let _: () = msg_send![engine, release];
            return Err(unavailable());
        }
        eprintln!("[voice] barge watch listening");
        Ok((engine as usize, input as usize))
    })?;
    Ok(BleedWatch {
        engine,
        input,
        interrupt,
    })
}

#[cfg(test)]
mod tests {
    use super::resample;

    #[test]
    fn resample_keeps_duration() {
        let input = vec![0.0, 1.0, 0.0, -1.0];
        let output = resample(&input, 22_050.0, 48_000.0);
        let expected = ((4.0_f64) * 48_000.0 / 22_050.0).round() as usize;
        assert_eq!(output.len(), expected);
    }
}
