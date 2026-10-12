// SPDX-License-Identifier: GPL-3.0-or-later
//! C ABI used by the Qt adapter. See `include/fooyin_tta.h` for ownership rules.
//! All decoder work stays on the calling thread; callbacks must not unwind.

use crate::{Decoder, StreamInfo};
use std::ffi::{c_char, c_void, CString};
use std::io::{self, Read, Seek, SeekFrom};
use std::mem::size_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TtaIo {
    pub context: *mut c_void,
    pub read: Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> i64>,
    pub seek: Option<unsafe extern "C" fn(*mut c_void, i64, i32) -> i64>,
}

struct CallbackInput(TtaIo);

impl Read for CallbackInput {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        // SAFETY: tta_open's caller guarantees callback/context validity until close.
        let count =
            unsafe { self.0.read.unwrap()(self.0.context, output.as_mut_ptr(), output.len()) };
        if count < 0 || count as u64 > output.len() as u64 {
            return Err(io::Error::other("input read failed"));
        }
        Ok(count as usize)
    }
}

impl Seek for CallbackInput {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let (offset, origin) = match position {
            SeekFrom::Start(value) => (
                i64::try_from(value).map_err(|_| io::Error::other("seek offset too large"))?,
                0,
            ),
            SeekFrom::Current(value) => (value, 1),
            SeekFrom::End(value) => (value, 2),
        };
        // SAFETY: same callback lifetime contract as read.
        let result = unsafe { self.0.seek.unwrap()(self.0.context, offset, origin) };
        if result < 0 {
            return Err(io::Error::other("input seek failed"));
        }
        Ok(result as u64)
    }
}

pub struct TtaHandle {
    decoder: Decoder<CallbackInput>,
    error: CString,
}

fn error_string(message: &str) -> CString {
    CString::new(message.replace('\0', "?")).expect("NULs removed")
}

unsafe fn write_error(buffer: *mut c_char, capacity: usize, message: &str) {
    if !buffer.is_null() && capacity > 0 {
        let count = message.len().min(capacity - 1);
        // SAFETY: caller provides capacity writable bytes, disjoint from message.
        unsafe {
            ptr::copy_nonoverlapping(message.as_ptr(), buffer.cast(), count);
            *buffer.add(count) = 0;
        }
    }
}

/// Open a seekable callback source. Returns null and a message on failure.
///
/// # Safety
/// Callbacks and context must remain valid until `tta_close`, must not unwind or
/// reenter this handle, and must obey the read/seek contract in the C header.
/// `info` must be writable. A nonnull error buffer must hold `error_capacity` bytes.
#[no_mangle]
pub unsafe extern "C" fn tta_open(
    io: TtaIo,
    info: *mut StreamInfo,
    error: *mut c_char,
    error_capacity: usize,
) -> *mut TtaHandle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if info.is_null() || io.read.is_none() || io.seek.is_none() {
            return Err("Invalid TTA callback arguments".to_owned());
        }
        let decoder = Decoder::new(CallbackInput(io)).map_err(|e| e.to_string())?;
        // SAFETY: caller provides a writable StreamInfo.
        unsafe {
            *info = decoder.info();
        }
        Ok(Box::into_raw(Box::new(TtaHandle {
            decoder,
            error: CString::default(),
        })))
    }));
    let message = match result {
        Ok(Ok(handle)) => {
            // SAFETY: caller's error buffer contract.
            unsafe {
                write_error(error, error_capacity, "");
            }
            return handle;
        }
        Ok(Err(message)) => message,
        Err(_) => "Rust TTA decoder panicked while opening input".to_owned(),
    };
    // SAFETY: caller's error buffer contract.
    unsafe {
        write_error(error, error_capacity, &message);
    }
    ptr::null_mut()
}

/// Free a handle; null is accepted.
///
/// # Safety
/// A nonnull handle must be live, exclusively owned, and closed exactly once.
#[no_mangle]
pub unsafe extern "C" fn tta_close(handle: *mut TtaHandle) {
    if !handle.is_null() {
        // SAFETY: ownership is transferred back by the caller.
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

/// Return interleaved, full-scale signed 32-bit PCM; count is channel samples.
/// Returns 0 at EOF, -1 on error. Errors remain latched until a seek.
///
/// # Safety
/// Handle must be live and exclusively accessed. Output must be aligned and
/// writable for `capacity` i32s, disjoint from the handle, and at most isize::MAX bytes.
#[no_mangle]
pub unsafe extern "C" fn tta_read(
    handle: *mut TtaHandle,
    output: *mut i32,
    capacity: usize,
) -> i64 {
    // SAFETY: caller guarantees a valid handle when nonnull.
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return -1;
    };
    if !handle.error.as_bytes().is_empty() {
        return -1;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        if capacity == 0 {
            return Ok(0);
        }
        if output.is_null() || capacity > isize::MAX as usize / size_of::<i32>() {
            return Err("Invalid TTA output buffer".to_owned());
        }
        // SAFETY: caller guarantees writable, aligned, exclusive output storage.
        let samples = unsafe { std::slice::from_raw_parts_mut(output, capacity) };
        let count = handle
            .decoder
            .read_samples(samples)
            .map_err(|e| e.to_string())?;
        let shift = 32 - handle.decoder.info().bits_per_sample;
        for value in &mut samples[..count] {
            *value = value.wrapping_shl(u32::from(shift));
        }
        Ok(count as i64)
    }));
    match result {
        Ok(Ok(count)) => count,
        Ok(Err(error)) => {
            handle.error = error_string(&error);
            -1
        }
        Err(_) => {
            handle.error = error_string("Rust TTA decoder panicked while decoding");
            -1
        }
    }
}

/// Seek in sample frames, clamping to EOF, and clear a previous error.
///
/// # Safety
/// A nonnull handle must be live and exclusively accessed.
#[no_mangle]
pub unsafe extern "C" fn tta_seek(handle: *mut TtaHandle, sample: u64) -> i32 {
    // SAFETY: caller's handle contract.
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return -1;
    };
    handle.decoder.seek(sample);
    handle.error = CString::default();
    0
}

/// Current position in sample frames, or zero for a null handle.
///
/// # Safety
/// A nonnull handle must be live and not concurrently modified.
#[no_mangle]
pub unsafe extern "C" fn tta_position(handle: *const TtaHandle) -> u64 {
    // SAFETY: caller's handle contract.
    unsafe { handle.as_ref() }.map_or(0, |h| u64::from(h.decoder.position()))
}

/// Error text remains valid until the next operation on the handle.
///
/// # Safety
/// A nonnull handle must be live and not concurrently modified.
#[no_mangle]
pub unsafe extern "C" fn tta_last_error(handle: *const TtaHandle) -> *const c_char {
    // SAFETY: caller's handle contract.
    unsafe { handle.as_ref() }.map_or(b"Invalid TTA handle\0".as_ptr().cast(), |h| {
        h.error.as_ptr()
    })
}
