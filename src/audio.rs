//! Sichere Rust-Hülle um den miniaudio-Shim (native/ma_shim.c).

use std::ffi::{c_char, c_int, c_void, CStr};
use std::ptr;

pub const SAMPLE_RATE: u32 = 48_000;

#[repr(C)]
struct RawCtx {
    _private: [u8; 0],
}
#[repr(C)]
struct RawStream {
    _private: [u8; 0],
}

type CaptureCb = extern "C" fn(*mut c_void, *const f32, u32);
type PlaybackCb = extern "C" fn(*mut c_void, *mut f32, u32);

extern "C" {
    fn ipt_result_desc(result: c_int) -> *const c_char;
    fn ipt_ctx_new(backend: c_int, out: *mut *mut RawCtx) -> c_int;
    fn ipt_ctx_free(ctx: *mut RawCtx);
    fn ipt_ctx_backend_name(ctx: *mut RawCtx) -> *const c_char;
    fn ipt_ctx_refresh(ctx: *mut RawCtx) -> c_int;
    fn ipt_ctx_device_count(ctx: *mut RawCtx, capture: c_int) -> u32;
    fn ipt_ctx_device_name(ctx: *mut RawCtx, capture: c_int, index: u32) -> *const c_char;
    fn ipt_stream_open(
        ctx: *mut RawCtx,
        capture: c_int,
        device_index: c_int,
        period_frames: u32,
        exclusive: c_int,
        user: *mut c_void,
        ccb: Option<CaptureCb>,
        pcb: Option<PlaybackCb>,
        out: *mut *mut RawStream,
    ) -> c_int;
    fn ipt_stream_start(s: *mut RawStream) -> c_int;
    fn ipt_stream_free(s: *mut RawStream);
    fn ipt_stream_info(
        s: *mut RawStream,
        period: *mut u32,
        periods: *mut u32,
        rate: *mut u32,
        exclusive: *mut c_int,
    );
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn check(r: c_int) -> Result<(), String> {
    if r == 0 {
        Ok(())
    } else {
        Err(format!("{} (Code {r})", cstr(unsafe { ipt_result_desc(r) })))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Backend {
    Auto,
    Wasapi,
    PulseAudio,
    Alsa,
    Jack,
}

impl Backend {
    fn code(self) -> c_int {
        match self {
            Backend::Auto => 0,
            Backend::Wasapi => 1,
            Backend::PulseAudio => 2,
            Backend::Alsa => 3,
            Backend::Jack => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Auto => "Automatisch",
            Backend::Wasapi => "WASAPI",
            Backend::PulseAudio => "PulseAudio / PipeWire",
            Backend::Alsa => "ALSA",
            Backend::Jack => "JACK / PipeWire-JACK",
        }
    }

    pub fn available() -> &'static [Backend] {
        if cfg!(windows) {
            &[Backend::Auto]
        } else {
            &[Backend::Auto, Backend::Jack, Backend::PulseAudio, Backend::Alsa]
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
}

pub struct Context {
    raw: *mut RawCtx,
}

// miniaudio-Kontexte dürfen von einem anderen Thread aus benutzt werden,
// solange nicht zwei Threads gleichzeitig darauf zugreifen (wir besitzen ihn exklusiv).
unsafe impl Send for Context {}

impl Context {
    pub fn new(backend: Backend) -> Result<Context, String> {
        let mut raw = ptr::null_mut();
        check(unsafe { ipt_ctx_new(backend.code(), &mut raw) })?;
        let ctx = Context { raw };
        check(unsafe { ipt_ctx_refresh(ctx.raw) })?;
        Ok(ctx)
    }

    pub fn backend_name(&self) -> String {
        cstr(unsafe { ipt_ctx_backend_name(self.raw) })
    }

    pub fn devices(&self, capture: bool) -> Vec<DeviceInfo> {
        let c = capture as c_int;
        let n = unsafe { ipt_ctx_device_count(self.raw, c) };
        (0..n)
            .map(|i| DeviceInfo {
                name: cstr(unsafe { ipt_ctx_device_name(self.raw, c, i) }),
            })
            .collect()
    }

    fn index_of(&self, capture: bool, name: Option<&str>) -> c_int {
        match name {
            None => -1,
            Some(n) => self
                .devices(capture)
                .iter()
                .position(|d| d.name == n)
                .map(|i| i as c_int)
                .unwrap_or(-1),
        }
    }

    /// Öffnet das Mikrofon. Der Callback bekommt Mono-Samples (48 kHz, f32).
    pub fn open_capture<F>(&self, device: Option<&str>, period: u32, exclusive: bool, cb: F) -> Result<Stream, String>
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        let boxed: Box<Box<dyn FnMut(&[f32]) + Send>> = Box::new(Box::new(cb));
        let user = Box::into_raw(boxed) as *mut c_void;
        let mut raw = ptr::null_mut();
        let r = unsafe {
            ipt_stream_open(
                self.raw,
                1,
                self.index_of(true, device),
                period,
                exclusive as c_int,
                user,
                Some(capture_trampoline),
                None,
                &mut raw,
            )
        };
        if let Err(e) = check(r) {
            drop(unsafe { Box::from_raw(user as *mut Box<dyn FnMut(&[f32]) + Send>) });
            return Err(e);
        }
        Ok(Stream { raw, user, kind: StreamKind::Capture })
    }

    /// Öffnet den Kopfhörer. Der Callback füllt Mono-Samples (48 kHz, f32).
    pub fn open_playback<F>(&self, device: Option<&str>, period: u32, exclusive: bool, cb: F) -> Result<Stream, String>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        let boxed: Box<Box<dyn FnMut(&mut [f32]) + Send>> = Box::new(Box::new(cb));
        let user = Box::into_raw(boxed) as *mut c_void;
        let mut raw = ptr::null_mut();
        let r = unsafe {
            ipt_stream_open(
                self.raw,
                0,
                self.index_of(false, device),
                period,
                exclusive as c_int,
                user,
                None,
                Some(playback_trampoline),
                &mut raw,
            )
        };
        if let Err(e) = check(r) {
            drop(unsafe { Box::from_raw(user as *mut Box<dyn FnMut(&mut [f32]) + Send>) });
            return Err(e);
        }
        Ok(Stream { raw, user, kind: StreamKind::Playback })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { ipt_ctx_free(self.raw) };
    }
}

extern "C" fn capture_trampoline(user: *mut c_void, input: *const f32, frames: u32) {
    if input.is_null() {
        return;
    }
    let cb = unsafe { &mut *(user as *mut Box<dyn FnMut(&[f32]) + Send>) };
    let data = unsafe { std::slice::from_raw_parts(input, frames as usize) };
    cb(data);
}

extern "C" fn playback_trampoline(user: *mut c_void, output: *mut f32, frames: u32) {
    if output.is_null() {
        return;
    }
    let cb = unsafe { &mut *(user as *mut Box<dyn FnMut(&mut [f32]) + Send>) };
    let data = unsafe { std::slice::from_raw_parts_mut(output, frames as usize) };
    cb(data);
}

enum StreamKind {
    Capture,
    Playback,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StreamInfo {
    pub period: u32,
    pub periods: u32,
    pub rate: u32,
    pub exclusive: bool,
}

impl StreamInfo {
    pub fn period_ms(&self) -> f32 {
        if self.rate == 0 {
            return 0.0;
        }
        self.period as f32 * 1000.0 / self.rate as f32
    }

    /// Grobe Schätzung, wie viel Verzögerung der Gerätepuffer selbst verursacht.
    pub fn device_latency_ms(&self, capture: bool) -> f32 {
        if capture {
            self.period_ms()
        } else {
            self.period_ms() * self.periods.max(1) as f32
        }
    }
}

pub struct Stream {
    raw: *mut RawStream,
    user: *mut c_void,
    kind: StreamKind,
}

unsafe impl Send for Stream {}

impl Stream {
    pub fn start(&self) -> Result<(), String> {
        check(unsafe { ipt_stream_start(self.raw) })
    }

    pub fn info(&self) -> StreamInfo {
        let (mut period, mut periods, mut rate, mut excl) = (0u32, 0u32, 0u32, 0 as c_int);
        unsafe { ipt_stream_info(self.raw, &mut period, &mut periods, &mut rate, &mut excl) };
        StreamInfo { period, periods, rate, exclusive: excl != 0 }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // Erst das Gerät stoppen (danach laufen keine Callbacks mehr), dann den Callback freigeben.
        unsafe { ipt_stream_free(self.raw) };
        unsafe {
            match self.kind {
                StreamKind::Capture => drop(Box::from_raw(self.user as *mut Box<dyn FnMut(&[f32]) + Send>)),
                StreamKind::Playback => drop(Box::from_raw(self.user as *mut Box<dyn FnMut(&mut [f32]) + Send>)),
            }
        }
    }
}
