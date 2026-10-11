use core_foundation::{base::TCFType, url::CFURL};
use core_foundation_sys::base::CFAllocatorRef;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::url::CFURLRef;
use core_graphics::base::{kCGBitmapByteOrder32Big, kCGImageAlphaPremultipliedLast};
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::image::CGImage;
use core_graphics::sys::CGImageRef;
use foreign_types::ForeignType;
use image::{DynamicImage, ImageBuffer, Rgba};
use std::{path::Path, ptr};

#[link(name = "QuickLook", kind = "framework")]
extern "C" {
    fn QLThumbnailImageCreate(
        allocator: CFAllocatorRef,
        url: CFURLRef,
        max_thumbnail_size: CGSize,
        options: CFDictionaryRef,
    ) -> CGImageRef;
}

pub fn generate_thumbnail(path: &Path, target_path: &Path) -> Result<bool, String> {
    if target_path.exists() {
        return Ok(true);
    }

    let cf_url = CFURL::from_path(path, false)
        .ok_or_else(|| format!("Failed to convert path to CFURL: {}", path.display()))?;

    let size = CGSize::new(512.0, 512.0);
    let cg_image_ref = unsafe {
        QLThumbnailImageCreate(ptr::null(), cf_url.as_concrete_TypeRef(), size, ptr::null())
    };

    if cg_image_ref.is_null() {
        return Ok(false);
    }

    let cg_image = unsafe { CGImage::from_ptr(cg_image_ref) };
    write_cgimage_to_png(&cg_image, target_path)
}

fn write_cgimage_to_png(image: &CGImage, target_path: &Path) -> Result<bool, String> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Ok(false);
    }

    let color_space = CGColorSpace::create_device_rgb();
    let mut context = CGContext::create_bitmap_context(
        None,
        width,
        height,
        8,
        width * 4,
        &color_space,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big,
    );

    let rect = CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(width as f64, height as f64),
    );
    context.draw_image(rect, image);
    let buffer = context.data().to_vec();

    let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width as u32, height as u32, buffer)
        .ok_or_else(|| "Failed to convert Quick Look image data".to_string())?;
    let dynamic_image = DynamicImage::ImageRgba8(image_buffer);
    dynamic_image
        .save(target_path)
        .map_err(|e| format!("Failed to save Quick Look thumbnail: {e}"))?;
    Ok(true)
}
