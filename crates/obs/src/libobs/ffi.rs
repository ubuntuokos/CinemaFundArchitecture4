//! Declarations of the libobs 33.0.1 C API that the binding uses (S0, S1).
//!
//! Transcribed from the OBS Studio 33.0.1 headers `libobs/obs.h`, `libobs/obs-defs.h`,
//! `libobs/obs-source.h`, `libobs/obs-nix-platform.h`, `libobs/graphics/vec2.h` and
//! `libobs/media-io/video-io.h` (Copyright (C) Lain Bailey and the
//! OBS Studio contributors, GPL-2.0-or-later; reused-code register in docs/licensing.md §5).
//! Only data declarations and function-pointer types: no code. `abi_tests` checks the struct
//! layouts and the constants against the installed headers when `CFA4_LIBOBS_DIR` has them.

#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_void};

/// `MAX_AV_PLANES` (media-io-defs.h).
pub const MAX_AV_PLANES: usize = 8;

/// `OBS_VIDEO_SUCCESS` and the other `obs_reset_video` results (obs-defs.h).
pub const OBS_VIDEO_SUCCESS: c_int = 0;
pub const OBS_VIDEO_FAIL: c_int = -1;
pub const OBS_VIDEO_NOT_SUPPORTED: c_int = -2;
pub const OBS_VIDEO_INVALID_PARAM: c_int = -3;
pub const OBS_VIDEO_CURRENTLY_ACTIVE: c_int = -4;
pub const OBS_VIDEO_MODULE_NOT_FOUND: c_int = -5;

/// `enum obs_nix_platform_type` (obs-nix-platform.h).
pub type obs_nix_platform_type = c_int;
pub const OBS_NIX_PLATFORM_INVALID: obs_nix_platform_type = 0;
pub const OBS_NIX_PLATFORM_X11_EGL: obs_nix_platform_type = 1;
pub const OBS_NIX_PLATFORM_WAYLAND: obs_nix_platform_type = 2;

/// `enum video_format` (video-io.h); only the values used.
pub type video_format = c_int;
pub const VIDEO_FORMAT_RGBA: video_format = 6;
pub const VIDEO_FORMAT_BGRA: video_format = 7;

/// `enum video_colorspace`.
pub type video_colorspace = c_int;
pub const VIDEO_CS_SRGB: video_colorspace = 3;

/// `enum video_range_type`.
pub type video_range_type = c_int;
pub const VIDEO_RANGE_FULL: video_range_type = 2;

/// `enum obs_scale_type` (obs.h).
pub type obs_scale_type = c_int;
pub const OBS_SCALE_BICUBIC: obs_scale_type = 2;

/// `struct obs_video_info` (obs.h).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct obs_video_info {
    pub graphics_module: *const c_char,
    pub fps_num: u32,
    pub fps_den: u32,
    pub base_width: u32,
    pub base_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub output_format: video_format,
    pub adapter: u32,
    pub gpu_conversion: bool,
    pub colorspace: video_colorspace,
    pub range: video_range_type,
    pub scale_type: obs_scale_type,
}

/// `struct video_data` (video-io.h).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct video_data {
    pub data: [*mut u8; MAX_AV_PLANES],
    pub linesize: [u32; MAX_AV_PLANES],
    pub timestamp: u64,
}

/// `struct video_scale_info` (video-io.h).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct video_scale_info {
    pub format: video_format,
    pub width: u32,
    pub height: u32,
    pub range: video_range_type,
    pub colorspace: video_colorspace,
}

/// Opaque `obs_scene_t`.
#[repr(C)]
pub struct obs_scene_t {
    _private: [u8; 0],
}

/// Opaque `obs_source_t`.
#[repr(C)]
pub struct obs_source_t {
    _private: [u8; 0],
}

/// Opaque `obs_sceneitem_t`.
#[repr(C)]
pub struct obs_sceneitem_t {
    _private: [u8; 0],
}

/// Opaque `obs_data_t`.
#[repr(C)]
pub struct obs_data_t {
    _private: [u8; 0],
}

/// `enum obs_source_type` (obs-source.h).
pub type obs_source_type = c_int;
pub const OBS_SOURCE_TYPE_INPUT: obs_source_type = 0;

/// Output flags (obs-source.h).
pub const OBS_SOURCE_VIDEO: u32 = 1 << 0;
pub const OBS_SOURCE_ASYNC: u32 = 1 << 2;
pub const OBS_SOURCE_ASYNC_VIDEO: u32 = OBS_SOURCE_ASYNC | OBS_SOURCE_VIDEO;
pub const OBS_SOURCE_DO_NOT_DUPLICATE: u32 = 1 << 7;
pub const OBS_SOURCE_CAP_DISABLED: u32 = 1 << 10;

/// `struct vec2` (graphics/vec2.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct vec2 {
    pub x: f32,
    pub y: f32,
}

/// `struct obs_sceneitem_crop` (obs.h).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct obs_sceneitem_crop {
    pub left: c_int,
    pub top: c_int,
    pub right: c_int,
    pub bottom: c_int,
}

/// `struct obs_source_frame` (obs.h).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct obs_source_frame {
    pub data: [*mut u8; MAX_AV_PLANES],
    pub linesize: [u32; MAX_AV_PLANES],
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub format: video_format,
    pub color_matrix: [f32; 16],
    pub full_range: bool,
    pub max_luminance: u16,
    pub color_range_min: [f32; 3],
    pub color_range_max: [f32; 3],
    pub flip: bool,
    pub flags: u8,
    pub trc: u8,
    pub refs: std::os::raw::c_long,
    pub prev_frame: bool,
}

/// A callback slot of `obs_source_info` that CFA4 leaves empty (NULL).
pub type Unused = Option<unsafe extern "C" fn()>;

/// `struct obs_source_info` (obs-source.h), every field, so `obs_register_source_s` gets the full
/// size it expects (libobs reads `version` and others by field, not only the bytes passed).
#[repr(C)]
pub struct obs_source_info {
    pub id: *const c_char,
    pub type_: obs_source_type,
    pub output_flags: u32,
    pub get_name: Option<unsafe extern "C" fn(type_data: *mut c_void) -> *const c_char>,
    pub create: Option<unsafe extern "C" fn(settings: *mut obs_data_t, source: *mut obs_source_t) -> *mut c_void>,
    pub destroy: Option<unsafe extern "C" fn(data: *mut c_void)>,
    pub get_width: Unused,
    pub get_height: Unused,
    pub get_defaults: Unused,
    pub get_properties: Unused,
    pub update: Unused,
    pub activate: Unused,
    pub deactivate: Unused,
    pub show: Unused,
    pub hide: Unused,
    pub video_tick: Unused,
    pub video_render: Unused,
    pub filter_video: Unused,
    pub filter_audio: Unused,
    pub enum_active_sources: Unused,
    pub save: Unused,
    pub load: Unused,
    pub mouse_click: Unused,
    pub mouse_move: Unused,
    pub mouse_wheel: Unused,
    pub focus: Unused,
    pub key_click: Unused,
    pub filter_remove: Unused,
    pub type_data: *mut c_void,
    pub free_type_data: Unused,
    pub audio_render: Unused,
    pub enum_all_sources: Unused,
    pub transition_start: Unused,
    pub transition_stop: Unused,
    pub get_defaults2: Unused,
    pub get_properties2: Unused,
    pub audio_mix: Unused,
    pub icon_type: c_int,
    pub media_play_pause: Unused,
    pub media_restart: Unused,
    pub media_stop: Unused,
    pub media_next: Unused,
    pub media_previous: Unused,
    pub media_get_duration: Unused,
    pub media_get_time: Unused,
    pub media_set_time: Unused,
    pub media_get_state: Unused,
    pub version: u32,
    pub unversioned_id: *const c_char,
    pub missing_files: Unused,
    pub video_get_color_space: Unused,
    pub filter_add: Unused,
    pub get_dark_icon: Unused,
    pub get_light_icon: Unused,
}

/// Raw video callback (`obs_add_raw_video_callback`).
pub type RawVideoCallback = unsafe extern "C" fn(param: *mut c_void, frame: *mut video_data);

pub type FnGetVersionString = unsafe extern "C" fn() -> *const c_char;
pub type FnSetNixPlatform = unsafe extern "C" fn(obs_nix_platform_type);
pub type FnSetNixPlatformDisplay = unsafe extern "C" fn(*mut c_void);
pub type FnStartup = unsafe extern "C" fn(locale: *const c_char, module_config_path: *const c_char, store: *mut c_void) -> bool;
pub type FnShutdown = unsafe extern "C" fn();
pub type FnResetVideo = unsafe extern "C" fn(ovi: *mut obs_video_info) -> c_int;
pub type FnSceneCreate = unsafe extern "C" fn(name: *const c_char) -> *mut obs_scene_t;
pub type FnSceneGetSource = unsafe extern "C" fn(scene: *const obs_scene_t) -> *mut obs_source_t;
pub type FnSceneRelease = unsafe extern "C" fn(scene: *mut obs_scene_t);
pub type FnSetOutputSource = unsafe extern "C" fn(channel: u32, source: *mut obs_source_t);
pub type FnAddRawVideoCallback = unsafe extern "C" fn(conversion: *const video_scale_info, callback: RawVideoCallback, param: *mut c_void);
pub type FnRemoveRawVideoCallback = unsafe extern "C" fn(callback: RawVideoCallback, param: *mut c_void);

pub type FnRegisterSource = unsafe extern "C" fn(info: *const obs_source_info, size: usize);
pub type FnSourceCreatePrivate = unsafe extern "C" fn(id: *const c_char, name: *const c_char, settings: *mut obs_data_t) -> *mut obs_source_t;
pub type FnSourceRelease = unsafe extern "C" fn(source: *mut obs_source_t);
pub type FnSourceOutputVideo = unsafe extern "C" fn(source: *mut obs_source_t, frame: *const obs_source_frame);
pub type FnSourceSetAsyncUnbuffered = unsafe extern "C" fn(source: *mut obs_source_t, unbuffered: bool);
pub type FnSceneAdd = unsafe extern "C" fn(scene: *mut obs_scene_t, source: *mut obs_source_t) -> *mut obs_sceneitem_t;
pub type FnSceneitemSetVec2 = unsafe extern "C" fn(item: *mut obs_sceneitem_t, v: *const vec2);
pub type FnSceneitemSetRot = unsafe extern "C" fn(item: *mut obs_sceneitem_t, rot_deg: f32);
pub type FnSceneitemSetCrop = unsafe extern "C" fn(item: *mut obs_sceneitem_t, crop: *const obs_sceneitem_crop);

/// `XOpenDisplay` / `XCloseDisplay` (libX11.so.6).
pub type FnXOpenDisplay = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;
pub type FnXCloseDisplay = unsafe extern "C" fn(display: *mut c_void) -> c_int;
/// `wl_display_connect` / `wl_display_disconnect` (libwayland-client.so.0).
pub type FnWlDisplayConnect = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;
pub type FnWlDisplayDisconnect = unsafe extern "C" fn(display: *mut c_void);

/// Human-readable meaning of an `obs_reset_video` result.
pub fn reset_video_error(code: c_int) -> &'static str {
    match code {
        OBS_VIDEO_FAIL => "generic failure",
        OBS_VIDEO_NOT_SUPPORTED => "the graphics adapter lacks the needed capabilities",
        OBS_VIDEO_INVALID_PARAM => "invalid video parameter",
        OBS_VIDEO_CURRENTLY_ACTIVE => "video is currently active",
        OBS_VIDEO_MODULE_NOT_FOUND => "graphics module (libobs-opengl) not found",
        _ => "unknown error",
    }
}

#[cfg(test)]
mod abi_tests {
    //! Layout checks against the C compiler's view of the installed libobs headers. They run
    //! when `$CFA4_LIBOBS_DIR/include/obs/obs.h` (default `/opt/cfa4/libobs-33.0.1`) and `cc`
    //! exist, and are skipped (with a note) otherwise; the libobs CI job always has them.
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_match_the_installed_headers() {
        let dir = std::env::var_os("CFA4_LIBOBS_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| crate::LIBOBS_DEFAULT_DIR.into());
        let include = dir.join("include/obs");
        if !include.join("obs.h").is_file() {
            eprintln!("skipped: no libobs headers in {}", include.display());
            return;
        }
        let out = std::env::temp_dir().join(format!("cfa4-obs-abi-{}", std::process::id()));
        let src = out.with_extension("c");
        let program = r#"
#include <stdio.h>
#include <stddef.h>
#include <obs.h>
#include <obs-nix-platform.h>
int main(void) {
  printf("%zu %zu %zu %zu %zu %zu\n", sizeof(struct obs_video_info), offsetof(struct obs_video_info, output_format),
    offsetof(struct obs_video_info, gpu_conversion), offsetof(struct obs_video_info, colorspace),
    offsetof(struct obs_video_info, scale_type), sizeof(struct video_data));
  printf("%zu %zu %zu\n", offsetof(struct video_data, linesize), offsetof(struct video_data, timestamp), sizeof(struct video_scale_info));
  printf("%d %d %d %d %d %d %d\n", (int)VIDEO_FORMAT_BGRA, (int)VIDEO_CS_SRGB, (int)VIDEO_RANGE_FULL, (int)OBS_SCALE_BICUBIC,
    (int)OBS_NIX_PLATFORM_X11_EGL, (int)OBS_NIX_PLATFORM_WAYLAND, OBS_VIDEO_MODULE_NOT_FOUND);
  printf("%zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct obs_source_frame), offsetof(struct obs_source_frame, timestamp),
    offsetof(struct obs_source_frame, format), offsetof(struct obs_source_frame, full_range), offsetof(struct obs_source_frame, max_luminance),
    offsetof(struct obs_source_frame, flip), offsetof(struct obs_source_frame, trc), offsetof(struct obs_source_frame, refs),
    offsetof(struct obs_source_frame, prev_frame));
  printf("%zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct obs_source_info), offsetof(struct obs_source_info, type_data),
    offsetof(struct obs_source_info, icon_type), offsetof(struct obs_source_info, version), offsetof(struct obs_source_info, get_light_icon),
    sizeof(struct vec2), sizeof(struct obs_sceneitem_crop));
  printf("%d %d %u %u %u\n", (int)VIDEO_FORMAT_RGBA, (int)OBS_SOURCE_TYPE_INPUT, (unsigned)OBS_SOURCE_ASYNC_VIDEO,
    (unsigned)OBS_SOURCE_DO_NOT_DUPLICATE, (unsigned)OBS_SOURCE_CAP_DISABLED);
  return 0;
}
"#;
        std::fs::write(&src, program).expect("write probe");
        let status = std::process::Command::new("cc").arg(&src).arg("-I").arg(&include).arg("-o").arg(&out).status();
        let Ok(status) = status else {
            eprintln!("skipped: no C compiler");
            return;
        };
        assert!(status.success(), "the probe compiles against {}", include.display());
        let printed = std::process::Command::new(&out).output().expect("run probe");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&out);
        let text = String::from_utf8_lossy(&printed.stdout);
        let expected = format!(
            "{} {} {} {} {} {}\n{} {} {}\n{} {} {} {} {} {} {}\n{} {} {} {} {} {} {} {} {}\n{} {} {} {} {} {} {}\n{} {} {} {} {}\n",
            size_of::<obs_video_info>(),
            offset_of!(obs_video_info, output_format),
            offset_of!(obs_video_info, gpu_conversion),
            offset_of!(obs_video_info, colorspace),
            offset_of!(obs_video_info, scale_type),
            size_of::<video_data>(),
            offset_of!(video_data, linesize),
            offset_of!(video_data, timestamp),
            size_of::<video_scale_info>(),
            VIDEO_FORMAT_BGRA,
            VIDEO_CS_SRGB,
            VIDEO_RANGE_FULL,
            OBS_SCALE_BICUBIC,
            OBS_NIX_PLATFORM_X11_EGL,
            OBS_NIX_PLATFORM_WAYLAND,
            OBS_VIDEO_MODULE_NOT_FOUND,
            size_of::<obs_source_frame>(),
            offset_of!(obs_source_frame, timestamp),
            offset_of!(obs_source_frame, format),
            offset_of!(obs_source_frame, full_range),
            offset_of!(obs_source_frame, max_luminance),
            offset_of!(obs_source_frame, flip),
            offset_of!(obs_source_frame, trc),
            offset_of!(obs_source_frame, refs),
            offset_of!(obs_source_frame, prev_frame),
            size_of::<obs_source_info>(),
            offset_of!(obs_source_info, type_data),
            offset_of!(obs_source_info, icon_type),
            offset_of!(obs_source_info, version),
            offset_of!(obs_source_info, get_light_icon),
            size_of::<vec2>(),
            size_of::<obs_sceneitem_crop>(),
            VIDEO_FORMAT_RGBA,
            OBS_SOURCE_TYPE_INPUT,
            OBS_SOURCE_ASYNC_VIDEO,
            OBS_SOURCE_DO_NOT_DUPLICATE,
            OBS_SOURCE_CAP_DISABLED,
        );
        assert_eq!(text, expected);
    }
}
