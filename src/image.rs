//! Image decoding through WIC (JPEG/PNG/BMP/WebP… whatever Windows can open).

use windows::{
    core::{Interface, GUID, PCWSTR},
    Win32::{
        Foundation::GENERIC_READ,
        Graphics::Imaging::*,
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
    },
};

use crate::wide;

/// Premultiplied BGRA, top-down, stride = w * 4.
#[derive(Clone)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

pub enum Source<'a> {
    File(&'a std::path::Path),
    Bytes(&'a [u8]),
}

/// Decode and "cover"-fit (scale to fill, crop the centre) to exactly `w`×`h`.
/// The calling thread must have COM initialised.
pub fn decode_cover(src: Source, w: u32, h: u32) -> Option<Image> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let decoder = match src {
            Source::File(p) => {
                let p = wide(&p.to_string_lossy());
                wic.CreateDecoderFromFilename(PCWSTR(p.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
                    .ok()?
            }
            Source::Bytes(b) => {
                let stream = wic.CreateStream().ok()?;
                stream.InitializeFromMemory(b).ok()?;
                wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?
            }
        };
        let frame = decoder.GetFrame(0).ok()?;
        let (mut sw, mut sh) = (0u32, 0u32);
        frame.GetSize(&mut sw, &mut sh).ok()?;
        if sw == 0 || sh == 0 || w == 0 || h == 0 {
            return None;
        }
        let k = (w as f64 / sw as f64).max(h as f64 / sh as f64);
        let dw = ((sw as f64 * k).ceil() as u32).max(w);
        let dh = ((sh as f64 * k).ceil() as u32).max(h);
        let scaler = wic.CreateBitmapScaler().ok()?;
        scaler.Initialize(&frame, dw, dh, WICBitmapInterpolationModeFant).ok()?;
        let clipper = wic.CreateBitmapClipper().ok()?;
        let rc = WICRect { X: ((dw - w) / 2) as i32, Y: ((dh - h) / 2) as i32, Width: w as i32, Height: h as i32 };
        clipper.Initialize(&scaler, &rc).ok()?;
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(
            &clipper,
            &GUID_WICPixelFormat32bppPBGRA as *const GUID,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .ok()?;
        let mut px = vec![0u8; (w * h * 4) as usize];
        conv.cast::<IWICBitmapSource>().ok()?.CopyPixels(std::ptr::null(), w * 4, &mut px).ok()?;
        Some(Image { w, h, px })
    }
}
