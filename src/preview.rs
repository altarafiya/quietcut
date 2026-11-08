//! Windows' native Media Engine renders video/audio on its own media threads.
#![allow(unsafe_op_in_unsafe_fn)]
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use windows::{
    Win32::{Media::MediaFoundation::*, System::Com::*},
    core::{BSTR, Interface, implement},
};
use windows_sys::Win32::Foundation::HWND;

#[implement(IMFMediaEngineNotify)]
struct Notify {
    seek: Arc<AtomicU64>,
    play: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
}
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _p1: usize, _p2: u32) -> windows::core::Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 as u32
            || event == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 as u32
        {
            self.ready.store(true, Ordering::Release);
        }
        if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.failed.store(true, Ordering::Release);
        }
        let _ = (&self.seek, &self.play);
        Ok(())
    }
}
pub struct Preview {
    engine: IMFMediaEngine,
    path: Option<std::path::PathBuf>,
    seek: Arc<AtomicU64>,
    play: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    pending: bool,
}
impl Preview {
    pub unsafe fn new(hwnd: HWND) -> windows::core::Result<Self> {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let seek = Arc::new(AtomicU64::new(0));
        let play = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let notify: IMFMediaEngineNotify = Notify {
            seek: seek.clone(),
            play: play.clone(),
            ready: ready.clone(),
            failed: failed.clone(),
        }
        .into();
        let mut attr = None;
        MFCreateAttributes(&mut attr, 3)?;
        let attr = attr.unwrap();
        attr.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
        attr.SetUINT64(&MF_MEDIA_ENGINE_PLAYBACK_HWND, hwnd as u64)?;
        let factory: IMFMediaEngineClassFactory =
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
        let engine = factory.CreateInstance(0, &attr)?;
        engine.SetMuted(true)?;
        Ok(Self {
            engine,
            path: None,
            seek,
            play,
            ready,
            failed,
            pending: false,
        })
    }
    pub unsafe fn seek(&mut self, path: &Path, position: f64, playing: bool) {
        self.seek.store(position.to_bits(), Ordering::Release);
        self.play.store(playing, Ordering::Release);
        if self.path.as_deref() != Some(path) {
            let _ = self.engine.Pause();
            self.ready.store(false, Ordering::Release);
            self.failed.store(false, Ordering::Release);
            self.pending = true;
            self.path = Some(path.to_owned());
            if self
                .engine
                .SetSource(&BSTR::from(path.to_string_lossy().as_ref()))
                .is_err()
            {
                self.failed.store(true, Ordering::Release);
            }
            let _ = self.engine.Load();
        } else if !self.pending {
            let _ = self.engine.SetCurrentTime(position);
            if playing {
                let _ = self.engine.Play();
            } else {
                let _ = self.engine.Pause();
            }
        }
    }
    pub unsafe fn tick(&mut self) {
        if self.pending && self.ready.load(Ordering::Acquire) {
            self.pending = false;
            let _ = self
                .engine
                .SetCurrentTime(f64::from_bits(self.seek.load(Ordering::Acquire)));
            if self.play.load(Ordering::Acquire) {
                let _ = self.engine.Play();
            }
        }
    }
    pub unsafe fn position(&self) -> Option<f64> {
        if self.pending || self.failed.load(Ordering::Acquire) {
            None
        } else {
            Some(self.engine.GetCurrentTime())
        }
    }
    pub unsafe fn pause(&self) {
        self.play.store(false, Ordering::Release);
        let _ = self.engine.Pause();
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn ready(&self) -> bool {
        !self.pending && !self.failed()
    }
    pub unsafe fn set_playing(&self, on: bool) {
        unsafe {
            if on {
                let _ = self.engine.Play();
            } else {
                let _ = self.engine.Pause();
            }
        }
    }
    pub unsafe fn resize(&self, width: i32, height: i32) {
        if let Ok(ex) = self.engine.cast::<IMFMediaEngineEx>() {
            let _ = ex.UpdateVideoStream(
                None,
                Some(&windows::Win32::Foundation::RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                }),
                None,
            );
        }
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}
