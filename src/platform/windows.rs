//! Windows 10 (x86_64) platform implementation.
//!
//! Process discovery uses the Toolhelp32 snapshot API because Windows has
//! no terminal foreground process group. Herdr treats the pane shell PID
//! as a stable synthetic process group id and reports the shell plus its
//! recursive descendants as the foreground job. Terminal foreground-group
//! precision is traded for process-tree coverage, which keeps agent
//! detection and shutdown working on Windows.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, GlobalFree, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;

use super::{ClipboardImage, ForegroundJob, ForegroundProcess, Signal};

const STILL_ACTIVE: u32 = 259;
const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;
const GMEM_MOVEABLE: u32 = 0x0002;

/// Upper bound on a clipboard global allocation Herdr copies from the
/// clipboard. Matches the official Windows clipboard-image allocation cap
/// (64 MiB); anything larger is rejected before the slice/copy.
const MAX_CLIPBOARD_GLOBAL_BYTES: usize = 64 * 1024 * 1024;

/// Clamps a clipboard `GlobalSize` result to a copyable length.
///
/// Zero-size and over-cap handles yield `None` so the caller skips the
/// slice/copy entirely instead of allocating attacker-chosen sizes.
fn clipboard_copy_len(global_size: usize, max_bytes: usize) -> Option<usize> {
    (global_size > 0 && global_size <= max_bytes).then_some(global_size)
}

/// Windows has no per-process file-descriptor soft limit to raise.
pub fn raise_server_nofile_limit() {}

#[derive(Debug, Clone)]
struct ProcessEntry {
    pid: u32,
    parent_pid: u32,
    exe_name: String,
}

fn snapshot_processes() -> Option<Vec<ProcessEntry>> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut entries: Vec<ProcessEntry> = Vec::new();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let exe_name = wide_field_to_string(&entry.szExeFile);
                entries.push(ProcessEntry {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    exe_name,
                });
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snapshot);
        Some(entries)
    }
}

fn wide_field_to_string(field: &[u16]) -> String {
    let len = field
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(field.len());
    String::from_utf16_lossy(&field[..len])
}

fn descendant_pids(root_pid: u32) -> Vec<u32> {
    let mut result = Vec::new();
    let Some(entries) = snapshot_processes() else {
        return result;
    };

    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for entry in &entries {
        children
            .entry(entry.parent_pid)
            .or_default()
            .push(entry.pid);
    }

    let mut queue: VecDeque<u32> = VecDeque::new();
    queue.push_back(root_pid);
    let mut seen = std::collections::HashSet::new();
    seen.insert(root_pid);
    while let Some(pid) = queue.pop_front() {
        result.push(pid);
        for &child in children.get(&pid).into_iter().flatten() {
            if seen.insert(child) {
                queue.push_back(child);
            }
        }
    }
    result
}

fn foreground_process_for(entry: &ProcessEntry) -> ForegroundProcess {
    ForegroundProcess {
        pid: entry.pid,
        name: entry.exe_name.clone(),
        argv0: Some(entry.exe_name.clone()),
        argv: None,
        cmdline: None,
    }
}

fn find_process(pid: u32) -> Option<ProcessEntry> {
    snapshot_processes()?
        .into_iter()
        .find(|entry| entry.pid == pid)
}

/// Reports the pane shell and its recursive descendants as one job.
///
/// Windows has no terminal foreground group, so the shell PID doubles as
/// a stable synthetic process group id. Agent detection matches the
/// process names in this tree the same way it matches foreground jobs on
/// Unix.
pub fn foreground_job(child_pid: u32) -> Option<ForegroundJob> {
    let entries = snapshot_processes()?;
    let by_pid: HashMap<u32, &ProcessEntry> =
        entries.iter().map(|entry| (entry.pid, entry)).collect();

    let mut processes = Vec::new();
    for pid in descendant_pids(child_pid) {
        if let Some(entry) = by_pid.get(&pid) {
            processes.push(foreground_process_for(entry));
        }
    }
    if processes.is_empty() {
        return None;
    }
    Some(ForegroundJob {
        process_group_id: child_pid,
        processes,
    })
}

/// Reports the tracked process itself as the leader of its synthetic group.
pub fn foreground_group_leader_job(process_group_id: u32) -> Option<ForegroundJob> {
    let entry = find_process(process_group_id)?;
    Some(ForegroundJob {
        process_group_id,
        processes: vec![foreground_process_for(&entry)],
    })
}

/// Returns the synthetic group id (the shell PID) while the process lives.
pub fn foreground_process_group_id(child_pid: u32) -> Option<u32> {
    process_exists(child_pid).then_some(child_pid)
}

pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    // Phase 1: OSC 7 from shell integrations still provides pane cwd.
    None
}

pub fn session_processes(child_pid: u32) -> Vec<u32> {
    descendant_pids(child_pid)
}

pub fn signal_processes(pids: &[u32], _signal: Signal) {
    for &pid in pids {
        terminate_process(pid);
    }
}

fn terminate_process(pid: u32) {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return;
        }
        TerminateProcess(handle, 1);
        CloseHandle(handle);
    }
}

pub fn process_exists(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut exit_code: u32 = 0;
        let queried = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        queried != 0 && exit_code == STILL_ACTIVE
    }
}

pub fn write_clipboard(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        let committed = (|| {
            if EmptyClipboard() == 0 {
                return false;
            }
            let handle = GlobalAlloc(GMEM_MOVEABLE, byte_len);
            if handle.is_null() {
                return false;
            }
            let target = GlobalLock(handle);
            if target.is_null() {
                GlobalFree(handle);
                return false;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target.cast::<u16>(), wide.len());
            GlobalUnlock(handle);
            if SetClipboardData(CF_UNICODETEXT, handle).is_null() {
                GlobalFree(handle);
                false
            } else {
                // Ownership moves to the clipboard after a successful call.
                true
            }
        })();
        CloseClipboard();
        committed
    }
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    let verb = wide_null("open");
    let target = wide_null(url);
    unsafe {
        // SW_SHOWNORMAL
        let result = ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        );
        let code = result as isize;
        if code <= 32 {
            return Err(std::io::Error::other(format!(
                "ShellExecuteW failed to open {url} (code {code})"
            )));
        }
    }
    Ok(())
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn read_clipboard_image() -> Option<ClipboardImage> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let image = (|| {
            if IsClipboardFormatAvailable(CF_DIB) == 0 {
                return None;
            }
            let handle = GetClipboardData(CF_DIB);
            if handle.is_null() {
                return None;
            }
            let data = GlobalLock(handle) as *const u8;
            if data.is_null() {
                return None;
            }
            // Cap the allocation size before building the slice: a clipboard
            // handle is foreign memory, and its size is not trusted.
            let len = match clipboard_copy_len(GlobalSize(handle), MAX_CLIPBOARD_GLOBAL_BYTES) {
                Some(len) => len,
                None => {
                    GlobalUnlock(handle);
                    return None;
                }
            };
            let dib = std::slice::from_raw_parts(data, len).to_vec();
            GlobalUnlock(handle);

            dib_to_png(&dib)
        })();
        CloseClipboard();
        image
    }
}

/// Returns the byte offset of the pixels in a clipboard DIB.
///
/// A DIB does not contain the file-header field used by a BMP file. The
/// pixel data starts after the bitmap header, any color masks, and any palette.
fn dib_pixel_offset(dib: &[u8]) -> Option<usize> {
    if dib.len() < 40 {
        return None;
    }

    let header_size = u32::from_le_bytes(dib[0..4].try_into().ok()?) as usize;
    if header_size < 40 || header_size > dib.len() {
        return None;
    }

    let bit_count = u16::from_le_bytes(dib[14..16].try_into().ok()?);
    let compression = u32::from_le_bytes(dib[16..20].try_into().ok()?);
    let colors_used = u32::from_le_bytes(dib[32..36].try_into().ok()?) as usize;

    let mask_bytes = match compression {
        3 => 12,
        6 => 16,
        _ => 0,
    };
    let palette_entries = if bit_count <= 8 {
        if colors_used != 0 {
            colors_used
        } else {
            1usize.checked_shl(bit_count.into())?
        }
    } else {
        0
    };

    header_size
        .checked_add(mask_bytes)?
        .checked_add(palette_entries.checked_mul(4)?)
        .filter(|offset| *offset <= dib.len())
}

/// Wraps a clipboard DIB payload in a BITMAPFILEHEADER and re-encodes as
/// PNG so downstream consumers get a normal image blob.
fn dib_to_png(dib: &[u8]) -> Option<ClipboardImage> {
    let pixel_offset = dib_pixel_offset(dib)?;

    // BITMAPFILEHEADER: type, size, reserved, reserved, offset.
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(&0x4d42u16.to_le_bytes());
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&(pixel_offset as u32 + 14).to_le_bytes());
    bmp.extend_from_slice(dib);

    let decoded = image::load_from_memory(&bmp).ok()?;
    let mut png_bytes = Vec::new();
    decoded
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .ok()?;

    Some(ClipboardImage {
        bytes: png_bytes,
        extension: "png",
    })
}

pub fn clipboard_image_read_support_hint() -> Option<&'static str> {
    None
}

pub fn show_desktop_notification(_title: &str, _body: Option<&str>) -> std::io::Result<bool> {
    // Phase 1: in-app toasts cover notifications; Windows toast
    // integration can layer on later.
    Ok(false)
}

pub(crate) fn encode_windows_conpty_fallback(key: &crate::input::TerminalKey) -> Option<Vec<u8>> {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

    let (virtual_key_code, virtual_scan_code, unicode, control_key_state) =
        if let Some(record) = key.windows_record() {
            (
                record.virtual_key_code,
                record.virtual_scan_code,
                record.unicode,
                record.control_key_state,
            )
        } else if key.code == KeyCode::Esc
            && key.modifiers.is_empty()
            && key.kind == KeyEventKind::Press
            && key.vt_bytes().is_none()
        {
            return Some(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_".to_vec());
        } else if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::SHIFT {
            (13, 28, 13, 16)
        } else {
            return None;
        };
    let key_down = key.kind != KeyEventKind::Release;
    let repeat_count = if key_down { key.repeat_count.max(1) } else { 1 };

    Some(
        format!(
            "\x1b[{virtual_key_code};{virtual_scan_code};{unicode};{};{control_key_state};{repeat_count}_",
            u8::from(key_down),
        )
        .into_bytes(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn clipboard_copy_len_rejects_zero_and_over_cap_sizes() {
        use super::{clipboard_copy_len, MAX_CLIPBOARD_GLOBAL_BYTES};

        assert_eq!(clipboard_copy_len(0, MAX_CLIPBOARD_GLOBAL_BYTES), None);
        assert_eq!(clipboard_copy_len(1, MAX_CLIPBOARD_GLOBAL_BYTES), Some(1));
        assert_eq!(
            clipboard_copy_len(MAX_CLIPBOARD_GLOBAL_BYTES, MAX_CLIPBOARD_GLOBAL_BYTES),
            Some(MAX_CLIPBOARD_GLOBAL_BYTES)
        );
        assert_eq!(
            clipboard_copy_len(MAX_CLIPBOARD_GLOBAL_BYTES + 1, MAX_CLIPBOARD_GLOBAL_BYTES),
            None
        );
        assert_eq!(
            clipboard_copy_len(usize::MAX, MAX_CLIPBOARD_GLOBAL_BYTES),
            None
        );
    }

    #[test]
    fn dib_pixel_offset_uses_header_masks_and_palette() {
        use super::dib_pixel_offset;

        let mut rgb = vec![0u8; 40 + 4];
        rgb[0..4].copy_from_slice(&40u32.to_le_bytes());
        rgb[14..16].copy_from_slice(&24u16.to_le_bytes());
        assert_eq!(dib_pixel_offset(&rgb), Some(40));

        let mut indexed = vec![0u8; 40 + 256 * 4 + 1];
        indexed[0..4].copy_from_slice(&40u32.to_le_bytes());
        indexed[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert_eq!(dib_pixel_offset(&indexed), Some(40 + 256 * 4));

        let mut bitfields = vec![0u8; 52 + 4];
        bitfields[0..4].copy_from_slice(&40u32.to_le_bytes());
        bitfields[14..16].copy_from_slice(&32u16.to_le_bytes());
        bitfields[16..20].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(dib_pixel_offset(&bitfields), Some(52));
    }

    #[test]
    fn windows_conpty_native_encoder_uses_canonical_phase_and_repeat_count() {
        let key = crate::input::TerminalKey::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::empty(),
        )
        .with_windows_record(crate::input::WindowsKeyRecord {
            key_down: true,
            repeat_count: 3,
            virtual_key_code: 27,
            virtual_scan_code: 1,
            unicode: 27,
            control_key_state: 0,
        });

        assert_eq!(
            super::encode_windows_conpty_fallback(&key),
            Some(b"\x1b[27;1;27;1;0;3_".to_vec())
        );
        let mut release = key.with_kind(crossterm::event::KeyEventKind::Release);
        release.repeat_count = 3;
        assert_eq!(
            super::encode_windows_conpty_fallback(&release),
            Some(b"\x1b[27;1;27;0;0;1_".to_vec())
        );
    }

    #[test]
    fn windows_conpty_native_encoder_preserves_semantic_escape_fallback() {
        let escape = crate::input::TerminalKey::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::empty(),
        );

        assert_eq!(
            super::encode_windows_conpty_fallback(&escape),
            Some(b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_".to_vec())
        );
        assert_eq!(
            super::encode_windows_conpty_fallback(
                &escape
                    .clone()
                    .with_kind(crossterm::event::KeyEventKind::Repeat),
            ),
            None
        );
        assert_eq!(
            super::encode_windows_conpty_fallback(
                &escape
                    .clone()
                    .with_kind(crossterm::event::KeyEventKind::Release),
            ),
            None
        );
        assert_eq!(
            super::encode_windows_conpty_fallback(&escape.clone().with_vt_bytes(vec![27])),
            None
        );
        assert_eq!(
            super::encode_windows_conpty_fallback(&crate::input::TerminalKey::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::ALT,
            ),),
            None
        );
    }

    #[test]
    fn windows_conpty_native_encoder_preserves_semantic_shift_enter_fallback() {
        let shift_enter = crate::input::TerminalKey::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::SHIFT,
        );

        assert_eq!(
            super::encode_windows_conpty_fallback(&shift_enter),
            Some(b"\x1b[13;28;13;1;16;1_".to_vec())
        );
        assert_eq!(
            super::encode_windows_conpty_fallback(&crate::input::TerminalKey::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::empty(),
            )),
            None
        );
    }
}
