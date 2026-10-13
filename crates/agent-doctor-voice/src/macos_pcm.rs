//! Play 16 kHz mono s16le through AudioQueue so a cloud reply can be stopped.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::types::{SpeechError, SpeechErrorCode};

#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioQueueBuffer {
    audio_data_bytes_capacity: u32,
    audio_data: *mut u8,
    audio_data_byte_size: u32,
    user_data: *mut c_void,
    packet_description_capacity: u32,
    packet_descriptions: *mut c_void,
    packet_description_count: u32,
}

type AudioQueueRef = *mut c_void;
type AudioQueueBufferRef = *mut AudioQueueBuffer;

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioQueueNewOutput(
        format: *const AudioStreamBasicDescription,
        callback: extern "C" fn(*mut c_void, AudioQueueRef, AudioQueueBufferRef),
        user_data: *mut c_void,
        callback_run_loop: *mut c_void,
        callback_run_loop_mode: *mut c_void,
        flags: u32,
        queue: *mut AudioQueueRef,
    ) -> i32;
    fn AudioQueueAllocateBuffer(
        queue: AudioQueueRef,
        buffer_size: u32,
        buffer: *mut AudioQueueBufferRef,
    ) -> i32;
    fn AudioQueueEnqueueBuffer(
        queue: AudioQueueRef,
        buffer: AudioQueueBufferRef,
        desc_count: u32,
        descs: *const c_void,
    ) -> i32;
    fn AudioQueueStart(queue: AudioQueueRef, start_time: *const c_void) -> i32;
    fn AudioQueueStop(queue: AudioQueueRef, immediate: u8) -> i32;
    fn AudioQueueDispose(queue: AudioQueueRef, immediate: u8) -> i32;
}

struct QueueState {
    pcm: Mutex<VecDeque<u8>>,
    finished: AtomicBool,
    drained: AtomicBool,
}

extern "C" fn fill_buffer(user: *mut c_void, queue: AudioQueueRef, buffer: AudioQueueBufferRef) {
    if user.is_null() || buffer.is_null() {
        return;
    }
    let state = unsafe { &*(user as *const QueueState) };
    let header = unsafe { &mut *buffer };
    let capacity = header.audio_data_bytes_capacity as usize;
    let mut copied = 0usize;
    if let Ok(mut pcm) = state.pcm.lock() {
        while copied + 1 < capacity {
            let Some(byte) = pcm.pop_front() else {
                break;
            };
            unsafe {
                *header.audio_data.add(copied) = byte;
            }
            copied += 1;
        }
    }
    if copied == 0 {
        if state.finished.load(Ordering::Relaxed) {
            state.drained.store(true, Ordering::Relaxed);
            return;
        }
        unsafe {
            ptr::write_bytes(header.audio_data, 0, capacity.min(640));
        }
        header.audio_data_byte_size = capacity.min(640) as u32;
    } else {
        header.audio_data_byte_size = copied as u32;
    }
    unsafe {
        AudioQueueEnqueueBuffer(queue, buffer, 0, ptr::null());
    }
}

pub(crate) fn play_pcm16_mono(
    pcm: &[u8],
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    if pcm.len() < 2 {
        return Ok(());
    }
    let state = Arc::new(QueueState {
        pcm: Mutex::new(pcm.iter().copied().collect()),
        finished: AtomicBool::new(true),
        drained: AtomicBool::new(false),
    });
    let format = AudioStreamBasicDescription {
        sample_rate: 16_000.0,
        format_id: 0x6C70_636D,
        format_flags: 4 | 8,
        bytes_per_packet: 2,
        frames_per_packet: 1,
        bytes_per_frame: 2,
        channels_per_frame: 1,
        bits_per_channel: 16,
        reserved: 0,
    };
    let mut queue: AudioQueueRef = ptr::null_mut();
    let status = unsafe {
        AudioQueueNewOutput(
            &format,
            fill_buffer,
            Arc::as_ptr(&state) as *mut c_void,
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            &mut queue,
        )
    };
    if status != 0 || queue.is_null() {
        return Err(SpeechError::new(
            SpeechErrorCode::Failed,
            "could not start cloud playback",
        ));
    }
    let mut buffers = [ptr::null_mut(), ptr::null_mut(), ptr::null_mut()];
    for slot in &mut buffers {
        let allocated = unsafe { AudioQueueAllocateBuffer(queue, 3200, slot) };
        if allocated != 0 || slot.is_null() {
            unsafe { AudioQueueDispose(queue, 1) };
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "could not start cloud playback",
            ));
        }
        fill_buffer(Arc::as_ptr(&state) as *mut c_void, queue, *slot);
    }
    let started = unsafe { AudioQueueStart(queue, ptr::null()) };
    if started != 0 {
        unsafe { AudioQueueDispose(queue, 1) };
        return Err(SpeechError::new(
            SpeechErrorCode::Failed,
            "could not start cloud playback",
        ));
    }
    let result = loop {
        if should_cancel() {
            break Err(SpeechError::new(
                SpeechErrorCode::Cancelled,
                "speech cancelled",
            ));
        }
        if state.drained.load(Ordering::Relaxed) {
            break Ok(());
        }
        thread::sleep(Duration::from_millis(30));
    };
    unsafe {
        AudioQueueStop(queue, 1);
        AudioQueueDispose(queue, 1);
    }
    result
}
