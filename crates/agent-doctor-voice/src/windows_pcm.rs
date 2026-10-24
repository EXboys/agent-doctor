//! Play 16 kHz mono s16le through waveOut so a cloud reply can be stopped.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use windows::core::PSTR;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    waveOutClose, waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader,
    waveOutWrite, CALLBACK_EVENT, HWAVEOUT, WAVEFORMATEX, WAVEHDR, WAVE_FORMAT_PCM, WAVE_MAPPER,
    WHDR_DONE, WHDR_INQUEUE,
};
use windows::Win32::Media::MMSYSERR_NOERROR;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::types::{SpeechError, SpeechErrorCode};

const BUFFER_BYTES: usize = 3200;
const BUFFER_COUNT: usize = 3;
/// Silence written while waiting for the next chunk, same length as on macOS.
const UNDERRUN_PAD: usize = 640;

struct Slot {
    data: Vec<u8>,
    header: WAVEHDR,
    prepared: bool,
    queued: bool,
}

/// Moves whole 16-bit samples only. A buffer ending on half a sample shifts every
/// later sample by one byte, which plays as loud hiss.
fn take_whole_samples(pcm: &mut VecDeque<u8>, out: &mut [u8]) -> usize {
    let count = pcm.len().min(out.len()) & !1;
    for (slot, byte) in out.iter_mut().zip(pcm.drain(..count)) {
        *slot = byte;
    }
    count
}

fn playback_failed() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Failed, "could not start cloud playback")
}

fn cancelled() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Cancelled, "speech cancelled")
}

pub(crate) fn play_pcm16_mono(
    pcm: &[u8],
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = tx.send(pcm.to_vec());
    drop(tx);
    play_pcm16_stream(&rx, should_cancel, &|| {})
}

/// Starts playing at the first chunk and keeps taking chunks until the sender is dropped.
/// `on_start` runs once the speakers have audio to play.
pub(crate) fn play_pcm16_stream(
    chunks: &Receiver<Vec<u8>>,
    should_cancel: &dyn Fn() -> bool,
    on_start: &dyn Fn(),
) -> Result<(), SpeechError> {
    let first = loop {
        if should_cancel() {
            return Err(cancelled());
        }
        match chunks.recv_timeout(Duration::from_millis(30)) {
            Ok(chunk) if chunk.is_empty() => continue,
            Ok(chunk) => break chunk,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    };
    let mut pcm: VecDeque<u8> = first.into_iter().collect();
    let mut finished = false;

    let event = unsafe { CreateEventW(None, false, false, None) }.map_err(|_| playback_failed())?;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: 16_000,
        nAvgBytesPerSec: 32_000,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut wave = HWAVEOUT::default();
    let opened = unsafe {
        waveOutOpen(
            Some(&mut wave),
            WAVE_MAPPER,
            &format,
            event.0 as usize,
            0,
            CALLBACK_EVENT,
        )
    };
    if opened != MMSYSERR_NOERROR {
        let _ = unsafe { CloseHandle(event) };
        return Err(playback_failed());
    }

    let mut slots: Vec<Slot> = (0..BUFFER_COUNT)
        .map(|_| Slot {
            data: vec![0u8; BUFFER_BYTES],
            header: WAVEHDR::default(),
            prepared: false,
            queued: false,
        })
        .collect();

    let result = (|| {
        // Prime the device before the speakers make a sound.
        while queue_ready(
            wave, &mut slots, &mut pcm, finished, /*allow_silence*/ false,
        )? > 0
        {}
        if !slots.iter().any(|slot| slot.queued) {
            // First chunk was shorter than one buffer; still send what we have.
            queue_ready(wave, &mut slots, &mut pcm, true, false)?;
        }
        if !slots.iter().any(|slot| slot.queued) {
            return Ok(());
        }
        on_start();

        loop {
            if should_cancel() {
                return Err(cancelled());
            }
            if !finished {
                loop {
                    match chunks.try_recv() {
                        Ok(chunk) => pcm.extend(chunk),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                    }
                }
            }
            reclaim_done(wave, &mut slots)?;
            queue_ready(wave, &mut slots, &mut pcm, finished, !finished)?;
            if finished && pcm.is_empty() && slots.iter().all(|slot| !slot.queued) {
                return Ok(());
            }
            let waited = unsafe { WaitForSingleObject(event, 30) };
            if waited != WAIT_OBJECT_0 && waited != WAIT_TIMEOUT {
                return Err(playback_failed());
            }
        }
    })();

    unsafe {
        let _ = waveOutReset(wave);
    }
    for slot in &mut slots {
        let _ = release_slot(wave, slot);
    }
    unsafe {
        let _ = waveOutClose(wave);
        let _ = CloseHandle(event);
    }
    result
}

fn reclaim_done(wave: HWAVEOUT, slots: &mut [Slot]) -> Result<(), SpeechError> {
    for slot in slots.iter_mut() {
        if !slot.queued {
            continue;
        }
        if slot.header.dwFlags & WHDR_DONE == 0 && slot.header.dwFlags & WHDR_INQUEUE != 0 {
            continue;
        }
        if slot.header.dwFlags & WHDR_DONE != 0 || slot.header.dwFlags & WHDR_INQUEUE == 0 {
            slot.queued = false;
            release_slot(wave, slot)?;
        }
    }
    Ok(())
}

fn release_slot(wave: HWAVEOUT, slot: &mut Slot) -> Result<(), SpeechError> {
    if !slot.prepared {
        return Ok(());
    }
    let status = unsafe {
        waveOutUnprepareHeader(
            wave,
            &mut slot.header,
            std::mem::size_of::<WAVEHDR>() as u32,
        )
    };
    slot.prepared = false;
    slot.header = WAVEHDR::default();
    if status != MMSYSERR_NOERROR {
        return Err(playback_failed());
    }
    Ok(())
}

/// Fills free slots from `pcm`. Returns how many buffers were submitted.
fn queue_ready(
    wave: HWAVEOUT,
    slots: &mut [Slot],
    pcm: &mut VecDeque<u8>,
    finished: bool,
    allow_silence: bool,
) -> Result<usize, SpeechError> {
    let mut submitted = 0usize;
    for slot in slots.iter_mut() {
        if slot.queued {
            continue;
        }
        let mut written = take_whole_samples(pcm, &mut slot.data);
        if written == 0 {
            if !allow_silence || finished {
                break;
            }
            slot.data[..UNDERRUN_PAD].fill(0);
            written = UNDERRUN_PAD;
        }
        submit(wave, slot, written)?;
        submitted += 1;
    }
    Ok(submitted)
}

fn submit(wave: HWAVEOUT, slot: &mut Slot, bytes: usize) -> Result<(), SpeechError> {
    debug_assert!(bytes > 0 && bytes <= slot.data.len());
    slot.header = WAVEHDR {
        lpData: PSTR(slot.data.as_mut_ptr()),
        dwBufferLength: bytes as u32,
        dwBytesRecorded: 0,
        dwUser: 0,
        dwFlags: 0,
        dwLoops: 0,
        lpNext: std::ptr::null_mut(),
        reserved: 0,
    };
    let prepared = unsafe {
        waveOutPrepareHeader(
            wave,
            &mut slot.header,
            std::mem::size_of::<WAVEHDR>() as u32,
        )
    };
    if prepared != MMSYSERR_NOERROR {
        slot.header = WAVEHDR::default();
        return Err(playback_failed());
    }
    slot.prepared = true;
    let written = unsafe {
        waveOutWrite(
            wave,
            &mut slot.header,
            std::mem::size_of::<WAVEHDR>() as u32,
        )
    };
    if written != MMSYSERR_NOERROR {
        let _ = release_slot(wave, slot);
        return Err(playback_failed());
    }
    slot.queued = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_never_split_a_sample() {
        let mut pcm: VecDeque<u8> = (0..10u8).collect();
        let mut out = [0u8; 3];
        assert_eq!(take_whole_samples(&mut pcm, &mut out), 2);
        assert_eq!(&out[..2], &[0, 1]);
        assert_eq!(pcm.front(), Some(&2));
        let mut rest = [0u8; 16];
        assert_eq!(take_whole_samples(&mut pcm, &mut rest), 8);
        assert_eq!(&rest[..8], &[2, 3, 4, 5, 6, 7, 8, 9]);
    }
}
