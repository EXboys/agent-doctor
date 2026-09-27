//! Windows: listen to the microphone while a reply is playing.
//!
//! Windows playback has no echo cancellation here, so the speakers leak into
//! the mic. [`barge::BleedGate`] learns that level first and only stops the
//! reply when a voice stays clearly louder.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{IInspectable, Interface};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Media::Audio::{
    AudioDeviceNodeCreationStatus, AudioFrameOutputNode, AudioGraph, AudioGraphCreationStatus,
    AudioGraphSettings,
};
use windows::Media::AudioBufferAccessMode;
use windows::Media::Capture::MediaCategory;
use windows::Media::Render::AudioRenderCategory;
use windows::Win32::System::WinRT::IMemoryBufferByteAccess;

use super::barge::{self, BargeAction, BleedGate};

pub(super) struct BleedWatch {
    graph: AudioGraph,
    token: EventRegistrationToken,
    interrupt: Arc<AtomicBool>,
}

impl BleedWatch {
    pub(super) fn interrupted(&self) -> bool {
        self.interrupt.load(Ordering::Relaxed)
    }

    pub(super) fn stop(self) {
        let _ = self.graph.Stop();
        let _ = self.graph.RemoveQuantumStarted(self.token);
        let _ = self.graph.Close();
    }
}

/// Starts the mic monitor. `None` means playback continues without it.
pub(super) fn start_bleed_watch() -> Option<BleedWatch> {
    match try_start() {
        Ok(watch) => {
            eprintln!("[voice] barge watch listening");
            Some(watch)
        }
        Err(detail) => {
            eprintln!("[voice] barge watch failed: {detail}");
            None
        }
    }
}

fn try_start() -> Result<BleedWatch, String> {
    let settings = AudioGraphSettings::Create(AudioRenderCategory::Speech)
        .map_err(|e| format!("graph settings: {e}"))?;
    let created = AudioGraph::CreateAsync(&settings)
        .and_then(|op| op.get())
        .map_err(|e| format!("create graph: {e}"))?;
    if created.Status().map_err(|e| e.to_string())? != AudioGraphCreationStatus::Success {
        return Err("audio graph was refused".into());
    }
    let graph = created.Graph().map_err(|e| e.to_string())?;

    let device = graph
        .CreateDeviceInputNodeAsync(MediaCategory::Speech)
        .and_then(|op| op.get())
        .map_err(|e| format!("microphone node: {e}"));
    let device = match device {
        Ok(device) => device,
        Err(detail) => {
            let _ = graph.Close();
            return Err(detail);
        }
    };
    if device.Status().ok() != Some(AudioDeviceNodeCreationStatus::Success) {
        let _ = graph.Close();
        return Err("microphone was refused".into());
    }
    let input = device.DeviceInputNode().map_err(|e| e.to_string())?;
    let frames = graph
        .CreateFrameOutputNode()
        .map_err(|e| format!("frame node: {e}"))?;
    input
        .AddOutgoingConnection(&frames)
        .map_err(|e| format!("connect microphone: {e}"))?;

    let encoding = graph.EncodingProperties().map_err(|e| e.to_string())?;
    let rate = encoding.SampleRate().unwrap_or(0);
    let channels = encoding.ChannelCount().unwrap_or(1).max(1);

    let interrupt = Arc::new(AtomicBool::new(false));
    let interrupt_for_graph = interrupt.clone();
    let state = Arc::new(Mutex::new((BleedGate::new(), 0.0_f32, 0_u64)));
    let handler = TypedEventHandler::<AudioGraph, IInspectable>::new(move |_, _| {
        let Some((rms, samples)) = read_rms(&frames) else {
            return Ok(());
        };
        let frame_ms = if rate > 0 {
            (samples as u64 / channels as u64) * 1000 / rate as u64
        } else {
            10
        }
        .max(1);
        let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
        let (gate, peak, window_ms) = &mut *guard;
        let action = gate.push(rms, frame_ms);
        *peak = peak.max(rms);
        *window_ms += frame_ms;
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
            interrupt_for_graph.store(true, Ordering::Relaxed);
        }
        Ok(())
    });
    let token = graph
        .QuantumStarted(&handler)
        .map_err(|e| format!("frame events: {e}"))?;
    if let Err(e) = graph.Start() {
        let _ = graph.RemoveQuantumStarted(token);
        let _ = graph.Close();
        return Err(format!("start graph: {e}"));
    }
    Ok(BleedWatch {
        graph,
        token,
        interrupt,
    })
}

/// Loudness of the newest mic frame, and how many float samples it held.
fn read_rms(frames: &AudioFrameOutputNode) -> Option<(f32, usize)> {
    let frame = frames.GetFrame().ok()?;
    let buffer = frame.LockBuffer(AudioBufferAccessMode::Read).ok()?;
    let reference = buffer.CreateReference().ok()?;
    let access: IMemoryBufferByteAccess = reference.cast().ok()?;
    let mut data: *mut u8 = std::ptr::null_mut();
    let mut capacity: u32 = 0;
    unsafe { access.GetBuffer(&mut data, &mut capacity) }.ok()?;
    let length = buffer.Length().ok()?.min(capacity) as usize;
    if data.is_null() || length < 4 {
        return None;
    }
    // Audio graphs deliver 32-bit float samples by default.
    let samples = unsafe { std::slice::from_raw_parts(data as *const f32, length / 4) };
    let rms = rms_of(samples);
    let count = samples.len();
    drop(access);
    let _ = reference.Close();
    let _ = buffer.Close();
    Some((rms, count))
}

fn rms_of(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|value| value * value).sum();
    (sum / samples.len() as f32).sqrt()
}
