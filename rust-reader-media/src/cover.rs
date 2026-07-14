//! Headless cover generation via a dedicated mpv instance using the `image`
//! video output (mpv >= 0.36). Blocking; call from a worker thread.

use crate::error::MediaError;
use libmpv_sys as mpv;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn cover_output_path(covers_dir: &Path, id: &str) -> PathBuf {
    covers_dir.join(format!("{id}.png"))
}

fn cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

pub fn generate_cover(input: &Path, output: &Path, timeout: Duration) -> Result<(), MediaError> {
    if !input.exists() {
        return Err(MediaError::Load("文件不存在".to_string()));
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("cover");
    // mpv's image VO writes `00000001.png`-style names into --vo-image-outdir
    // (the outfile-template option is gone in current mpv), so each run gets
    // a private temp dir next to the target: concurrent generations cannot
    // collide and the final rename stays on one filesystem.
    let tmp_dir = parent.join(format!(".{stem}.cover-tmp-{}", std::process::id()));
    // Clear a stale dir from a crashed run, then start fresh.
    std::fs::remove_dir_all(&tmp_dir).ok();
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| MediaError::Load(format!("封面临时目录创建失败: {e}")))?;
    let result = run_mpv_image_grab(input, &tmp_dir, timeout)
        .and_then(|()| find_produced_image(&tmp_dir))
        .and_then(|produced| {
            std::fs::rename(&produced, output)
                .map_err(|e| MediaError::Load(format!("封面写入失败: {e}")))
        });
    std::fs::remove_dir_all(&tmp_dir).ok();
    result
}

fn run_mpv_image_grab(input: &Path, out_dir: &Path, timeout: Duration) -> Result<(), MediaError> {
    // SAFETY: mpv_create has no preconditions.
    let handle = unsafe { mpv::mpv_create() };
    if handle.is_null() {
        return Err(MediaError::Init("mpv_create 返回空句柄".into()));
    }
    let result = run_mpv_image_grab_with_handle(handle, input, out_dir, timeout);
    // SAFETY: handle is valid and owned by us; this blocks until the core is
    // torn down, so no event wait can outlive it.
    unsafe { mpv::mpv_terminate_destroy(handle) };
    result
}

fn run_mpv_image_grab_with_handle(
    handle: *mut mpv::mpv_handle,
    input: &Path,
    out_dir: &Path,
    timeout: Duration,
) -> Result<(), MediaError> {
    let out_dir = out_dir.to_string_lossy().to_string();
    // All options are set before mpv_initialize: setting VO options afterwards
    // is not reliable. `ao=null` additionally guards against AO init hangs
    // seen with some external sound cards; `aid=no` already deselects audio.
    for (k, v) in [
        ("vo", "image"),
        ("vo-image-format", "png"),
        ("vo-image-outdir", out_dir.as_str()),
        ("frames", "1"),
        ("start", "10%"),
        ("terminal", "no"),
        ("aid", "no"),
        ("ao", "null"),
    ] {
        let (k, v) = (cstring(k), cstring(v));
        // SAFETY: handle is a valid mpv handle; k/v are valid NUL-terminated
        // strings that outlive the call.
        let rc = unsafe { mpv::mpv_set_option_string(handle, k.as_ptr(), v.as_ptr()) };
        if rc < 0 {
            return Err(MediaError::Init(format!("设置 {k:?} 失败: {rc}")));
        }
    }
    // SAFETY: handle is valid and not yet initialized.
    let rc = unsafe { mpv::mpv_initialize(handle) };
    if rc < 0 {
        return Err(MediaError::Init(format!("mpv_initialize 失败: {rc}")));
    }
    let src = input
        .to_str()
        .ok_or_else(|| MediaError::Load("路径包含非 UTF-8 字符".into()))?;
    let args = [cstring("loadfile"), cstring(src)];
    let mut ptrs = [args[0].as_ptr(), args[1].as_ptr(), std::ptr::null()];
    // SAFETY: handle is valid; ptrs is a NULL-terminated array of valid
    // NUL-terminated strings that outlive the synchronous call.
    let rc = unsafe { mpv::mpv_command(handle, ptrs.as_mut_ptr()) };
    if rc < 0 {
        return Err(MediaError::Load(format!("loadfile 失败: {rc}")));
    }
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() > deadline {
            return Err(MediaError::Load("封面生成超时".into()));
        }
        // SAFETY: handle is valid; the returned event pointer is owned by mpv
        // and valid until the next mpv_wait_event call on this handle.
        let ev = unsafe { mpv::mpv_wait_event(handle, 0.5) };
        if ev.is_null() {
            continue;
        }
        // SAFETY: event is a valid pointer returned by mpv_wait_event.
        let id = unsafe { (*ev).event_id };
        if id == mpv::mpv_event_id_MPV_EVENT_SHUTDOWN {
            break;
        }
        if id == mpv::mpv_event_id_MPV_EVENT_END_FILE {
            // SAFETY: for MPV_EVENT_END_FILE, event data points to a valid
            // mpv_event_end_file owned by mpv for the duration of the event.
            let reason = unsafe {
                let d = (*ev).data as *mut mpv::mpv_event_end_file;
                if d.is_null() {
                    0
                } else {
                    (*d).reason
                }
            };
            if reason as u32 == mpv::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR {
                return Err(MediaError::Load("解码失败".into()));
            }
            break;
        }
    }
    Ok(())
}

/// Returns the single `*.png` the image VO dropped into `dir`.
fn find_produced_image(dir: &Path) -> Result<PathBuf, MediaError> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| MediaError::Load(format!("封面目录不可读: {e}")))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
        .collect();
    candidates.sort();
    candidates
        .pop()
        .ok_or_else(|| MediaError::Load("未生成封面图像".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn cover_output_path_uses_covers_dir_and_id() {
        let p = cover_output_path(&PathBuf::from("/data/covers"), "abc123");
        assert_eq!(p, PathBuf::from("/data/covers/abc123.png"));
    }

    #[test]
    fn generate_cover_reports_missing_input() {
        let err = generate_cover(
            std::path::Path::new("/definitely/not/here.mp4"),
            std::path::Path::new("/tmp/out.png"),
            std::time::Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(matches!(err, MediaError::Load(_)));
    }
}
