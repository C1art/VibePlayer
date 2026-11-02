use std::ptr;

use ffmpeg_sys_next as sys;

use super::string_from;

/// Reads every key/value pair of a dictionary in insertion order.
///
/// # Safety
/// `dict` must be null or a valid dictionary that isn't modified during the call.
pub(super) unsafe fn entries(dict: *const sys::AVDictionary) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut entry: *const sys::AVDictionaryEntry = ptr::null();
    loop {
        // SAFETY: the caller guarantees `dict` is valid or null; `entry` is null or the previous
        // entry returned for this dictionary.
        entry = unsafe { sys::av_dict_get(dict, c"".as_ptr(), entry, sys::AV_DICT_IGNORE_SUFFIX) };
        // SAFETY: a non-null entry points into the dictionary, which outlives this loop.
        let Some(current) = (unsafe { entry.as_ref() }) else {
            return entries;
        };
        // SAFETY: entry keys and values are NUL-terminated strings owned by the dictionary.
        let (key, value) = unsafe { (string_from(current.key), string_from(current.value)) };
        if let (Some(key), Some(value)) = (key, value) {
            entries.push((key, value));
        }
    }
}

/// Looks up one key, case-insensitively.
///
/// # Safety
/// `dict` must be null or a valid dictionary.
pub(super) unsafe fn get(dict: *const sys::AVDictionary, key: &std::ffi::CStr) -> Option<String> {
    // SAFETY: the caller guarantees `dict`; `key` is NUL-terminated.
    let entry = unsafe { sys::av_dict_get(dict, key.as_ptr(), ptr::null(), 0).as_ref() }?;
    // SAFETY: the value is a NUL-terminated string owned by the dictionary.
    unsafe { string_from(entry.value) }
}

/// Copies all entries of `source` into `*destination`, allocating it if null.
///
/// # Safety
/// `destination` must point to a valid dictionary pointer; `source` must be null or valid.
pub(super) unsafe fn copy(
    destination: *mut *mut sys::AVDictionary,
    source: *const sys::AVDictionary,
) {
    // SAFETY: guaranteed by the caller. A failed copy leaves partial metadata, which is harmless.
    unsafe { sys::av_dict_copy(destination, source, 0) };
}
