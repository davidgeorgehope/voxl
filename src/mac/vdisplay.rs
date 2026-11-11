//! A virtual display sized to the viewer (CoreGraphics' CGVirtualDisplay, the private
//! API macOS's own Sidecar-style features and Chromium's remoting tests use). It is
//! created when a client asks (`vd,W,H,scale`), captured instead of the real display,
//! and removed when that client leaves or voxl exits; the real display's mode is never
//! touched. It is made the main display (menu bar, Dock, new windows) for as long as it
//! exists, with an app-only configuration that macOS undoes when voxl quits.

use super::ffi::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, ClassType};
use objc2_foundation::{NSArray, NSObject, NSString};
use parking_lot::Mutex;
use std::time::{Duration, Instant};

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}
unsafe impl objc2::Encode for CGSize {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGConfigureDisplayOrigin(config: CGDisplayConfigRef, display: u32, x: i32, y: i32) -> i32;
}

/// A live virtual display; removed when dropped.
pub struct VirtualDisplay {
    obj: Retained<AnyObject>,
    pub id: u32,
    /// Size in pixels (what is streamed).
    pub width: u32,
    pub height: u32,
    pub hidpi: bool,
}
// The object is only created, kept and released; CoreGraphics serves it on the main queue.
unsafe impl Send for VirtualDisplay {}

static CURRENT: Mutex<Option<VirtualDisplay>> = Mutex::new(None);

fn class(name: &str) -> Result<&'static AnyClass, String> {
    let c = std::ffi::CString::new(name).unwrap();
    AnyClass::get(&c).ok_or_else(|| format!("{name} is not available on this macOS"))
}

impl VirtualDisplay {
    /// `width`x`height` pixels; `hidpi` draws at 2x (points = pixels / 2).
    pub fn create(width: u32, height: u32, hidpi: bool) -> Result<Self, String> {
        // Whole, even point sizes: macOS offers no Retina variant of an odd one.
        let k = if hidpi { 2 } else { 1 };
        let (width, height) = (width / (2 * k) * 2 * k, height / (2 * k) * 2 * k);
        unsafe {
            let desc: Retained<AnyObject> = {
                let c = class("CGVirtualDisplayDescriptor")?;
                let a: *mut AnyObject = msg_send![c, alloc];
                Retained::from_raw(msg_send![a, init]).ok_or("descriptor init failed")?
            };
            let queue = dispatch2::DispatchQueue::main();
            let _: () = msg_send![&*desc, setQueue: &*queue as *const _ as *const AnyObject];
            let name = NSString::from_str("voxl");
            let _: () = msg_send![&*desc, setName: &*name];
            let _: () = msg_send![&*desc, setMaxPixelsWide: width];
            let _: () = msg_send![&*desc, setMaxPixelsHigh: height];
            // ~110 pixels per inch at 1x: macOS picks text scaling from this.
            let mm = |px: u32| px as f64 / k as f64 / 110.0 * 25.4;
            let _: () = msg_send![&*desc, setSizeInMillimeters: CGSize { width: mm(width), height: mm(height) }];
            let _: () = msg_send![&*desc, setVendorID: super::VD_VENDOR];
            let _: () = msg_send![&*desc, setProductID: 0x786cu32];
            let _: () = msg_send![&*desc, setSerialNum: 1u32];
            let obj: Retained<AnyObject> = {
                let c = class("CGVirtualDisplay")?;
                let a: *mut AnyObject = msg_send![c, alloc];
                Retained::from_raw(msg_send![a, initWithDescriptor: &*desc])
                    .ok_or("CGVirtualDisplay init failed")?
            };
            let settings: Retained<AnyObject> = {
                let c = class("CGVirtualDisplaySettings")?;
                let a: *mut AnyObject = msg_send![c, alloc];
                Retained::from_raw(msg_send![a, init]).ok_or("settings init failed")?
            };
            let _: () = msg_send![&*settings, setHiDPI: hidpi as u32];
            // Modes are given in pixels; with HiDPI macOS adds the half-size Retina
            // variant of each, which is picked below.
            let mk = |w: u32, h: u32| -> Result<Retained<NSObject>, String> {
                let c = class("CGVirtualDisplayMode")?;
                let a: *mut AnyObject = msg_send![c, alloc];
                let m: *mut AnyObject = msg_send![a, initWithWidth: w, height: h, refreshRate: 60.0f64];
                Retained::from_raw(m as *mut NSObject).ok_or_else(|| "mode init failed".to_string())
            };
            let modes = if hidpi {
                NSArray::from_retained_slice(&[mk(width / k, height / k)?, mk(width, height)?])
            } else {
                NSArray::from_retained_slice(&[mk(width, height)?])
            };
            let _: () = msg_send![&*settings, setModes: &*modes];
            let ok: bool = msg_send![&*obj, applySettings: &*settings];
            if !ok {
                return Err(format!("virtual display {width}x{height} settings refused"));
            }
            let id: u32 = msg_send![&*obj, displayID];
            // It shows up among the active displays shortly.
            let t = Instant::now();
            while !super::active_displays().contains(&id) && t.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(20));
            }
            if !super::active_displays().contains(&id) {
                return Err("virtual display did not come online".into());
            }
            // Main display while it exists (app-only: undone when voxl exits), in the
            // Retina mode when asked for.
            let mut cfg: CGDisplayConfigRef = std::ptr::null_mut();
            if CGBeginDisplayConfiguration(&mut cfg) == 0 {
                CGConfigureDisplayOrigin(cfg, id, 0, 0);
                if hidpi {
                    let key = kCGDisplayShowDuplicateLowResolutionModes;
                    let opts = objc2_core_foundation::CFDictionary::from_slices(
                        &[&*(key as *const objc2_core_foundation::CFString)],
                        &[objc2_core_foundation::CFBoolean::new(true)],
                    );
                    let all = CGDisplayCopyAllDisplayModes(id, &*opts as *const _ as CFDictionaryRef);
                    if !all.is_null() {
                        for i in 0..CFArrayGetCount(all) {
                            let m = CFArrayGetValueAtIndex(all, i) as CGDisplayModeRef;
                            if CGDisplayModeGetPixelWidth(m) == width as usize
                                && CGDisplayModeGetWidth(m) == (width / k) as usize
                            {
                                CGConfigureDisplayWithDisplayMode(cfg, id, m, std::ptr::null());
                                break;
                            }
                        }
                        CFRelease(all as _);
                    }
                }
                if CGCompleteDisplayConfiguration(cfg, kCGConfigureForAppOnly) != 0 {
                    log::warn!("could not make the virtual display the main one");
                }
            }
            let b = CGDisplayBounds(id);
            log::info!(
                "virtual display {id}: {width}x{height} pixels ({}x{} points{})",
                b.size.width,
                b.size.height,
                if hidpi { ", HiDPI" } else { "" }
            );
            Ok(VirtualDisplay { obj, id, width, height, hidpi })
        }
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        log::info!("removing virtual display {}", self.id);
        let _ = &self.obj;
    }
}

/// Puts up the virtual display `req` asks for (None: remove it) and returns its ID.
pub fn apply(req: Option<(u32, u32, bool)>) -> Option<u32> {
    let mut cur = CURRENT.lock();
    let round = |x: u32, d: bool| if d { x / 4 * 4 } else { x / 2 * 2 };
    let same = matches!((&*cur, req), (Some(v), Some((w, h, d))) if v.width == round(w, d) && v.height == round(h, d) && v.hidpi == d);
    if !same {
        // Remove the old one first (and give macOS a moment to settle).
        if cur.take().is_some() {
            std::thread::sleep(Duration::from_millis(300));
        }
        if let Some((w, h, d)) = req {
            match VirtualDisplay::create(w, h, d) {
                Ok(v) => *cur = Some(v),
                Err(e) => log::warn!("virtual display: {e}"),
            }
        }
    }
    let r = cur.as_ref().map(|v| (v.id, v.width, v.height));
    super::set_virtual(r);
    r.map(|v| v.0)
}

pub fn available() -> bool {
    class("CGVirtualDisplay").is_ok()
}

#[allow(dead_code)]
fn _assert_class_type() {
    let _ = NSObject::class();
}
