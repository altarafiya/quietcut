//! Bounded, audio-only FFmpeg decoding into native WASAPI. No preview video render.
use quietcut::engine::{LEVEL_FILTER, Media, Tools};
use std::{
    io::Read,
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use windows::Win32::{Media::Audio::*, System::Com::*};
struct Shared {
    cancel: AtomicBool,
    start: AtomicBool,
    pub ready: AtomicBool,
    pub running: AtomicBool,
    pub finished: AtomicBool,
    position: AtomicU64,
    child: Mutex<Option<Child>>,
    error: Mutex<Option<String>>,
}
pub struct LiveAudio {
    state: Arc<Shared>,
}
impl LiveAudio {
    pub fn prepare(tools: Tools, media: Arc<Media>, start: f64, end: f64, level: bool) -> Self {
        let state = Arc::new(Shared {
            cancel: AtomicBool::new(false),
            start: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            running: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            position: AtomicU64::new(start.to_bits()),
            child: Mutex::new(None),
            error: Mutex::new(None),
        });
        let s = state.clone();
        thread::spawn(move || {
            if let Err(e) = unsafe { stream(&s, &tools, &media, start, end, level) } {
                if !s.cancel.load(Ordering::Acquire) {
                    *s.error.lock().unwrap() = Some(e);
                }
            }
            if let Some(child) = s.child.lock().unwrap().as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            s.finished.store(true, Ordering::Release);
        });
        Self { state }
    }
    pub fn start(&self) {
        self.state.start.store(true, Ordering::Release);
    }
    pub fn ready(&self) -> bool {
        self.state.ready.load(Ordering::Acquire)
    }
    pub fn running(&self) -> bool {
        self.state.running.load(Ordering::Acquire)
    }
    pub fn finished(&self) -> bool {
        self.state.finished.load(Ordering::Acquire)
    }
    pub fn position(&self) -> f64 {
        f64::from_bits(self.state.position.load(Ordering::Acquire))
    }
    pub fn error(&self) -> Option<String> {
        self.state.error.lock().unwrap().clone()
    }
    pub fn stop(&self) {
        self.state.cancel.store(true, Ordering::Release);
        if let Some(c) = self.state.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
}
impl Drop for LiveAudio {
    fn drop(&mut self) {
        self.stop();
    }
}
unsafe fn stream(
    s: &Shared,
    tools: &Tools,
    media: &Media,
    start: f64,
    end: f64,
    level: bool,
) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = (|| -> windows::core::Result<()> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let client: IAudioClient = device.Activate(CLSCTX_INPROC_SERVER, None)?;
            let format = WAVEFORMATEX {
                wFormatTag: 3,
                nChannels: 2,
                nSamplesPerSec: 48000,
                nAvgBytesPerSec: 384000,
                nBlockAlign: 8,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                600_000,
                0,
                &format,
                None,
            )?;
            let render: IAudioRenderClient = client.GetService()?;
            let capacity = client.GetBufferSize()?;
            let filter = if level {
                format!("aresample=48000:async=1:first_pts=0,{LEVEL_FILTER},apad")
            } else {
                "aresample=48000:async=1:first_pts=0,apad".into()
            };
            let mut child = Command::new(&tools.ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-nostdin",
                    "-vn",
                    "-ss",
                    &format!("{start:.9}"),
                    "-i",
                ])
                .arg(&media.path)
                .args([
                    "-t",
                    &format!("{:.9}", end - start),
                    "-map",
                    "0:a:0",
                    "-af",
                    &filter,
                    "-ac",
                    "2",
                    "-ar",
                    "48000",
                    "-f",
                    "f32le",
                    "pipe:1",
                ])
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| {
                    windows::core::Error::from_hresult(windows::core::HRESULT(0x80004005u32 as i32))
                })?;
            let mut output = child.stdout.take().unwrap();
            *s.child.lock().unwrap() = Some(child);
            let mut bytes = [0u8; 8192];
            let mut submitted = 0u64;
            let mut started = false;
            let mut eof = false;
            while !eof && !s.cancel.load(Ordering::Acquire) {
                let mut n = 0;
                while n < bytes.len() {
                    match output.read(&mut bytes[n..]) {
                        Ok(0) => {
                            eof = true;
                            break;
                        }
                        Ok(count) => n += count,
                        Err(_) => {
                            eof = true;
                            break;
                        }
                    }
                }
                let frames = (n / 8) as u32;
                let mut offset = 0u32;
                while offset < frames && !s.cancel.load(Ordering::Acquire) {
                    let padding = client.GetCurrentPadding()?;
                    s.position.store(
                        (start + submitted.saturating_sub(padding as u64) as f64 / 48000.)
                            .min(end)
                            .to_bits(),
                        Ordering::Release,
                    );
                    let available = capacity.saturating_sub(padding);
                    if available == 0 {
                        thread::sleep(Duration::from_millis(4));
                        continue;
                    }
                    let count = available.min(frames - offset);
                    let dest = render.GetBuffer(count)?;
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr().add(offset as usize * 8),
                        dest,
                        count as usize * 8,
                    );
                    render.ReleaseBuffer(count, 0)?;
                    offset += count;
                    submitted += count as u64;
                    if !started {
                        s.ready.store(true, Ordering::Release);
                        while !s.start.load(Ordering::Acquire) && !s.cancel.load(Ordering::Acquire)
                        {
                            thread::sleep(Duration::from_millis(4));
                        }
                        if s.cancel.load(Ordering::Acquire) {
                            break;
                        }
                        client.Start()?;
                        started = true;
                        s.running.store(true, Ordering::Release);
                    }
                }
            }
            while started && !s.cancel.load(Ordering::Acquire) {
                let padding = client.GetCurrentPadding()?;
                s.position.store(
                    (start + submitted.saturating_sub(padding as u64) as f64 / 48000.)
                        .min(end)
                        .to_bits(),
                    Ordering::Release,
                );
                if padding == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(4));
            }
            if started {
                let _ = client.Stop();
            }
            let _ = client.Reset();
            if !s.cancel.load(Ordering::Acquire) {
                if let Some(c) = s.child.lock().unwrap().as_mut() {
                    if !c.wait().map(|x| x.success()).unwrap_or(false) {
                        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                            0x80004005u32 as i32,
                        )));
                    }
                }
            }
            Ok(())
        })();
        CoUninitialize();
        result.map_err(|e| format!("Audio preview could not start: {e}"))
    }
}
