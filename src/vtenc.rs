//! Hardware H.264 encoding with Apple's VideoToolbox (macOS).
//!
//! Same contract as the x264 encoder: I420 planes in, Annex-B out (SPS/PPS repeated on
//! keyframes), no B-frames, keyframes only when asked for. Frames are encoded
//! synchronously: `VTCompressionSessionCompleteFrames` blocks until the output
//! callback has run, so one `encode` call yields that frame's bytes.

#![allow(non_upper_case_globals, non_snake_case)]

use std::ffi::c_void;
use std::ptr;

use parking_lot::Mutex;

use crate::encoder::{EncoderConfig, Planes};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFDictionaryRef = *const c_void;
type OSStatus = i32;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

const CMTIME_VALID: u32 = 1;
fn cmtime(ms: i64) -> CMTime {
    CMTime { value: ms, timescale: 1000, flags: CMTIME_VALID, epoch: 0 }
}
fn cmtime_invalid() -> CMTime {
    CMTime { value: 0, timescale: 0, flags: 0, epoch: 0 }
}

type OutputCb = extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, *const c_void);

#[link(name = "VideoToolbox", kind = "framework")]
extern "C" {
    fn VTCompressionSessionCreate(
        alloc: CFAllocatorRef,
        w: i32,
        h: i32,
        codec: u32,
        spec: CFDictionaryRef,
        src_attrs: CFDictionaryRef,
        data_alloc: CFAllocatorRef,
        cb: OutputCb,
        refcon: *mut c_void,
        out: *mut *mut c_void,
    ) -> OSStatus;
    fn VTSessionSetProperty(s: *mut c_void, key: CFStringRef, v: CFTypeRef) -> OSStatus;
    fn VTCompressionSessionPrepareToEncodeFrames(s: *mut c_void) -> OSStatus;
    fn VTCompressionSessionEncodeFrame(
        s: *mut c_void,
        img: *mut c_void,
        pts: CMTime,
        dur: CMTime,
        props: CFDictionaryRef,
        frame_refcon: *mut c_void,
        flags: *mut u32,
    ) -> OSStatus;
    fn VTCompressionSessionCompleteFrames(s: *mut c_void, until: CMTime) -> OSStatus;
    fn VTCompressionSessionInvalidate(s: *mut c_void);

    static kVTCompressionPropertyKey_RealTime: CFStringRef;
    static kVTCompressionPropertyKey_ProfileLevel: CFStringRef;
    static kVTProfileLevel_H264_High_AutoLevel: CFStringRef;
    static kVTCompressionPropertyKey_AllowFrameReordering: CFStringRef;
    static kVTCompressionPropertyKey_MaxKeyFrameInterval: CFStringRef;
    static kVTCompressionPropertyKey_AverageBitRate: CFStringRef;
    static kVTCompressionPropertyKey_DataRateLimits: CFStringRef;
    static kVTCompressionPropertyKey_Quality: CFStringRef;
    static kVTCompressionPropertyKey_ExpectedFrameRate: CFStringRef;
    static kVTCompressionPropertyKey_ColorPrimaries: CFStringRef;
    static kVTCompressionPropertyKey_TransferFunction: CFStringRef;
    static kVTCompressionPropertyKey_YCbCrMatrix: CFStringRef;
    static kVTEncodeFrameOptionKey_ForceKeyFrame: CFStringRef;
    static kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: CFStringRef;
    static kVTVideoEncoderSpecification_EnableLowLatencyRateControl: CFStringRef;
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        a: CFAllocatorRef,
        w: usize,
        h: usize,
        fmt: u32,
        attrs: CFDictionaryRef,
        out: *mut *mut c_void,
    ) -> i32;
    fn CVPixelBufferLockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(pb: *mut c_void, i: usize) -> *const u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(pb: *mut c_void, i: usize) -> usize;
    static kCVImageBufferColorPrimaries_ITU_R_709_2: CFStringRef;
    static kCVImageBufferTransferFunction_ITU_R_709_2: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_601_4: CFStringRef;
}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetDataBuffer(sb: *const c_void) -> *mut c_void;
    fn CMSampleBufferGetFormatDescription(sb: *const c_void) -> *const c_void;
    fn CMSampleBufferGetSampleAttachmentsArray(sb: *const c_void, create: bool) -> *const c_void;
    fn CMBlockBufferGetDataLength(bb: *mut c_void) -> usize;
    fn CMBlockBufferCopyDataBytes(bb: *mut c_void, off: usize, len: usize, dst: *mut u8) -> OSStatus;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        fd: *const c_void,
        idx: usize,
        ptr: *mut *const u8,
        size: *mut usize,
        count: *mut usize,
        nal_len: *mut i32,
    ) -> OSStatus;
    static kCMSampleAttachmentKey_NotSync: CFStringRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(c: CFTypeRef);
    fn CFNumberCreate(a: CFAllocatorRef, ty: isize, v: *const c_void) -> CFTypeRef;
    fn CFArrayCreate(a: CFAllocatorRef, v: *const CFTypeRef, n: isize, cb: *const c_void) -> CFTypeRef;
    fn CFArrayGetCount(a: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(a: *const c_void, i: isize) -> CFTypeRef;
    fn CFDictionaryCreate(
        a: CFAllocatorRef,
        k: *const CFTypeRef,
        v: *const CFTypeRef,
        n: isize,
        kcb: *const c_void,
        vcb: *const c_void,
    ) -> CFDictionaryRef;
    fn CFDictionaryGetValue(d: *const c_void, k: CFTypeRef) -> CFTypeRef;
    fn CFBooleanGetValue(b: CFTypeRef) -> u8;
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;
    static kCFTypeArrayCallBacks: u8;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

const K_CMVIDEOCODEC_H264: u32 = u32::from_be_bytes(*b"avc1");
const K_CVPIXFMT_NV12_VIDEO: u32 = u32::from_be_bytes(*b"420v");
const CFNUM_SINT32: isize = 3;
const CFNUM_FLOAT64: isize = 13;

unsafe fn num_i32(v: i32) -> CFTypeRef {
    CFNumberCreate(ptr::null(), CFNUM_SINT32, &v as *const i32 as *const c_void)
}
unsafe fn num_f64(v: f64) -> CFTypeRef {
    CFNumberCreate(ptr::null(), CFNUM_FLOAT64, &v as *const f64 as *const c_void)
}
unsafe fn dict(k: &[CFTypeRef], v: &[CFTypeRef]) -> CFDictionaryRef {
    CFDictionaryCreate(
        ptr::null(),
        k.as_ptr(),
        v.as_ptr(),
        k.len() as isize,
        &kCFTypeDictionaryKeyCallBacks as *const u8 as *const c_void,
        &kCFTypeDictionaryValueCallBacks as *const u8 as *const c_void,
    )
}

#[derive(Default)]
struct Output {
    data: Vec<u8>,
    key: bool,
    err: Option<OSStatus>,
}

pub struct Vt {
    s: *mut c_void,
    pb: *mut c_void,
    out: Box<Mutex<Output>>,
    force_key: CFDictionaryRef,
    pts: i64,
    fps: u32,
    pub width: u32,
    pub height: u32,
}

unsafe impl Send for Vt {}

extern "C" fn on_output(refcon: *mut c_void, _f: *mut c_void, status: OSStatus, _flags: u32, sb: *const c_void) {
    let out = unsafe { &*(refcon as *const Mutex<Output>) };
    let mut o = out.lock();
    if status != 0 || sb.is_null() {
        o.err = Some(status);
        return;
    }
    unsafe {
        let att = CMSampleBufferGetSampleAttachmentsArray(sb, false);
        let mut key = true;
        if !att.is_null() && CFArrayGetCount(att) > 0 {
            let d = CFArrayGetValueAtIndex(att, 0);
            let ns = CFDictionaryGetValue(d, kCMSampleAttachmentKey_NotSync);
            key = ns.is_null() || CFBooleanGetValue(ns) == 0;
        }
        o.key = key;
        if key {
            let fd = CMSampleBufferGetFormatDescription(sb);
            let mut count = 0usize;
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                fd, 0, ptr::null_mut(), ptr::null_mut(), &mut count, ptr::null_mut(),
            );
            for i in 0..count {
                let (mut p, mut n) = (ptr::null::<u8>(), 0usize);
                if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                    fd, i, &mut p, &mut n, ptr::null_mut(), ptr::null_mut(),
                ) == 0
                {
                    o.data.extend_from_slice(&[0, 0, 0, 1]);
                    o.data.extend_from_slice(std::slice::from_raw_parts(p, n));
                }
            }
        }
        let bb = CMSampleBufferGetDataBuffer(sb);
        let len = CMBlockBufferGetDataLength(bb);
        let mut avcc = vec![0u8; len];
        CMBlockBufferCopyDataBytes(bb, 0, len, avcc.as_mut_ptr());
        // AVCC (4-byte big-endian lengths) -> Annex-B start codes.
        let mut i = 0;
        while i + 4 <= len {
            let n = u32::from_be_bytes([avcc[i], avcc[i + 1], avcc[i + 2], avcc[i + 3]]) as usize;
            i += 4;
            if i + n > len {
                break;
            }
            o.data.extend_from_slice(&[0, 0, 0, 1]);
            o.data.extend_from_slice(&avcc[i..i + n]);
            i += n;
        }
    }
}

/// x264 CRF (lower = better) to VideoToolbox quality (0..1, higher = better).
fn quality_for(crf: f32) -> f64 {
    (1.0 - (crf as f64 - 10.0) / 40.0).clamp(0.3, 0.95)
}

impl Vt {
    pub fn new(cfg: &EncoderConfig) -> Result<Self, String> {
        unsafe {
            let out: Box<Mutex<Output>> = Box::default();
            let spec = dict(
                &[
                    kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder,
                    kVTVideoEncoderSpecification_EnableLowLatencyRateControl,
                ],
                &[kCFBooleanTrue, kCFBooleanTrue],
            );
            let mut s = ptr::null_mut();
            let st = VTCompressionSessionCreate(
                ptr::null(),
                cfg.width as i32,
                cfg.height as i32,
                K_CMVIDEOCODEC_H264,
                spec,
                ptr::null(),
                ptr::null(),
                on_output,
                &*out as *const Mutex<Output> as *mut c_void,
                &mut s,
            );
            CFRelease(spec);
            if st != 0 || s.is_null() {
                return Err(format!("VTCompressionSessionCreate failed: {st}"));
            }
            let set = |k: CFStringRef, v: CFTypeRef| {
                let r = VTSessionSetProperty(s, k, v);
                if r != 0 {
                    log::debug!("VTSessionSetProperty failed: {r}");
                }
            };
            set(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
            set(kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel);
            set(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
            let n = num_i32(i32::MAX);
            set(kVTCompressionPropertyKey_MaxKeyFrameInterval, n);
            CFRelease(n);
            let n = num_i32(cfg.fps as i32);
            set(kVTCompressionPropertyKey_ExpectedFrameRate, n);
            CFRelease(n);
            set(kVTCompressionPropertyKey_ColorPrimaries, kCVImageBufferColorPrimaries_ITU_R_709_2);
            set(kVTCompressionPropertyKey_TransferFunction, kCVImageBufferTransferFunction_ITU_R_709_2);
            set(kVTCompressionPropertyKey_YCbCrMatrix, kCVImageBufferYCbCrMatrix_ITU_R_601_4);
            let mut pb = ptr::null_mut();
            if CVPixelBufferCreate(
                ptr::null(),
                cfg.width as usize,
                cfg.height as usize,
                K_CVPIXFMT_NV12_VIDEO,
                ptr::null(),
                &mut pb,
            ) != 0
            {
                VTCompressionSessionInvalidate(s);
                CFRelease(s);
                return Err("CVPixelBufferCreate failed".into());
            }
            let force_key = dict(&[kVTEncodeFrameOptionKey_ForceKeyFrame], &[kCFBooleanTrue]);
            let mut v = Vt {
                s,
                pb,
                out,
                force_key,
                pts: 0,
                fps: cfg.fps.max(1),
                width: cfg.width,
                height: cfg.height,
            };
            v.set_rate(cfg.crf, cfg.max_kbps);
            VTCompressionSessionPrepareToEncodeFrames(s);
            log::info!("VideoToolbox H.264 encoder {}x{}", cfg.width, cfg.height);
            Ok(v)
        }
    }

    pub fn set_rate(&mut self, crf: f32, max_kbps: u32) {
        self.set_rate_buf(crf, max_kbps, max_kbps / self.fps * 5 / 4);
    }

    /// Quality follows the CRF; `max_kbps` caps the average and `buf_kbit` the burst
    /// (VideoToolbox's DataRateLimits: bytes per window).
    pub fn set_rate_buf(&mut self, crf: f32, max_kbps: u32, buf_kbit: u32) {
        unsafe {
            let q = num_f64(quality_for(crf));
            VTSessionSetProperty(self.s, kVTCompressionPropertyKey_Quality, q);
            CFRelease(q);
            if max_kbps > 0 {
                let br = num_i32((max_kbps as i64 * 1000).min(i32::MAX as i64) as i32);
                VTSessionSetProperty(self.s, kVTCompressionPropertyKey_AverageBitRate, br);
                CFRelease(br);
                // Bytes allowed per one second window, plus a short burst window.
                let b1 = num_i32((max_kbps as i64 * 125).min(i32::MAX as i64) as i32);
                let s1 = num_f64(1.0);
                let burst = buf_kbit.max(max_kbps / self.fps).max(1) as i64 * 125;
                let b2 = num_i32(burst.min(i32::MAX as i64) as i32);
                let s2 = num_f64(buf_kbit as f64 / max_kbps as f64);
                let vals = [b1, s1, b2, s2];
                let arr = CFArrayCreate(
                    ptr::null(),
                    vals.as_ptr(),
                    4,
                    &kCFTypeArrayCallBacks as *const u8 as *const c_void,
                );
                VTSessionSetProperty(self.s, kVTCompressionPropertyKey_DataRateLimits, arr);
                CFRelease(arr);
                for v in vals {
                    CFRelease(v);
                }
            }
        }
    }

    pub fn encode(
        &mut self,
        p: &Planes,
        ts_ms: i64,
        force_idr: bool,
        out: &mut Vec<u8>,
    ) -> Result<bool, String> {
        let (w, h) = (self.width as usize, self.height as usize);
        unsafe {
            CVPixelBufferLockBaseAddress(self.pb, 0);
            let y = CVPixelBufferGetBaseAddressOfPlane(self.pb, 0) as *mut u8;
            let ys = CVPixelBufferGetBytesPerRowOfPlane(self.pb, 0);
            for r in 0..h {
                ptr::copy_nonoverlapping(p.y.as_ptr().add(r * p.stride_y), y.add(r * ys), w);
            }
            let uv = CVPixelBufferGetBaseAddressOfPlane(self.pb, 1) as *mut u8;
            let uvs = CVPixelBufferGetBytesPerRowOfPlane(self.pb, 1);
            let cw = w / 2;
            for r in 0..h / 2 {
                let (u, v) = (&p.u[r * p.stride_uv..], &p.v[r * p.stride_uv..]);
                let d = std::slice::from_raw_parts_mut(uv.add(r * uvs), cw * 2);
                for c in 0..cw {
                    d[2 * c] = u[c];
                    d[2 * c + 1] = v[c];
                }
            }
            CVPixelBufferUnlockBaseAddress(self.pb, 0);

            self.pts = ts_ms.max(self.pts + 1);
            *self.out.lock() = Output::default();
            let st = VTCompressionSessionEncodeFrame(
                self.s,
                self.pb,
                cmtime(self.pts),
                cmtime_invalid(),
                if force_idr { self.force_key } else { ptr::null() },
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if st != 0 {
                return Err(format!("VTCompressionSessionEncodeFrame failed: {st}"));
            }
            VTCompressionSessionCompleteFrames(self.s, cmtime(self.pts));
        }
        let mut o = self.out.lock();
        if let Some(e) = o.err {
            return Err(format!("VideoToolbox encode error {e}"));
        }
        out.append(&mut o.data);
        Ok(o.key)
    }
}

impl Drop for Vt {
    fn drop(&mut self) {
        unsafe {
            VTCompressionSessionCompleteFrames(self.s, cmtime_invalid());
            VTCompressionSessionInvalidate(self.s);
            CFRelease(self.s);
            CFRelease(self.pb);
            CFRelease(self.force_key);
        }
    }
}
