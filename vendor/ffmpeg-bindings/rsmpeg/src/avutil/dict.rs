use crate::{error::Result, ffi, shared::*};

use std::{
    ffi::{CStr, CString},
    fmt::Debug,
    os::raw::c_void,
    ptr::{self, NonNull},
};

wrap_ref_mut!(AVDictionary: ffi::AVDictionary);

/// The `av_dict_set()`-family flags that hand the ownership of the key and value
/// pointers over to the dictionary.
///
/// This API cannot honour them: the pointers come from `&CStr`s that are only
/// borrowed for the duration of the call, not from `av_malloc()`. Letting
/// libavutil take them over makes the dictionary reference — and eventually
/// `av_free()` — memory its caller still owns, which is a use-after-free and a
/// double free waiting to happen.
///
/// They are masked away instead, which is what `av_dict_parse_string()` does
/// with them too (`/* ignore STRDUP flags */` in libavutil/dict.c).
const OWNERSHIP_FLAGS: u32 = ffi::AV_DICT_DONT_STRDUP_KEY | ffi::AV_DICT_DONT_STRDUP_VAL;

/// Drop the flags this API cannot honour, see [`OWNERSHIP_FLAGS`].
fn safe_flags(flags: u32) -> i32 {
    (flags & !OWNERSHIP_FLAGS) as i32
}

impl AVDictionary {
    /// Create a dictionary holding the single pair `key`/`value`.
    ///
    /// `flags` takes the `AV_DICT_*` values. `AV_DICT_DONT_STRDUP_KEY` and
    /// `AV_DICT_DONT_STRDUP_VAL` are ignored: they would hand the ownership of
    /// the strings over, and this API always keeps its own copies. Most callers
    /// pass `0`.
    pub fn new(key: &CStr, value: &CStr, flags: u32) -> Self {
        // Since AVDictionary is a non-null pointer to ffi::AVDictionary.
        // Without a new macro `wrap_nullable`, we cannot new a Self containing
        // null pointer.
        let mut dict = ptr::null_mut();
        unsafe { ffi::av_dict_set(&mut dict, key.as_ptr(), value.as_ptr(), safe_flags(flags)) }
            .upgrade()
            .unwrap();
        unsafe { Self::from_raw(NonNull::new(dict).unwrap()) }
    }

    /// Create a dictionary holding the single pair `key`/`value`, with `value`
    /// stored as its decimal representation.
    pub fn new_int(key: &CStr, value: i64, flags: u32) -> Self {
        let mut dict = ptr::null_mut();
        unsafe { ffi::av_dict_set_int(&mut dict, key.as_ptr(), value, safe_flags(flags)) }
            .upgrade()
            .unwrap();
        unsafe { Self::from_raw(NonNull::new(dict).unwrap()) }
    }

    /// Parse `str` into a new dictionary.
    ///
    /// `key_val_sep` and `pairs_sep` are NUL-terminated lists of the characters
    /// that separate a key from its value and one pair from the next. Returns
    /// `None` when there is nothing to parse or when parsing fails, without
    /// reporting which pair was at fault.
    ///
    /// A string holding no pairs has no representation to produce, see
    /// [`Self::parse_string()`] to merge into a dictionary that exists.
    pub fn from_string(
        str: &CStr,
        key_val_sep: &CStr,
        pairs_sep: &CStr,
        flags: u32,
    ) -> Option<Self> {
        let mut dict = ptr::null_mut();
        unsafe {
            ffi::av_dict_parse_string(
                &mut dict,
                str.as_ptr(),
                key_val_sep.as_ptr(),
                pairs_sep.as_ptr(),
                safe_flags(flags),
            )
        }
        .upgrade()
        .ok()?;
        // Parsing a string that holds no pairs succeeds without allocating a
        // dictionary, so there may be nothing to wrap.
        Some(unsafe { Self::from_raw(NonNull::new(dict)?) })
    }

    /// Store `key`, replacing the value it had, and give the dictionary back.
    ///
    /// Takes `self` by value because adding an entry invalidates every entry
    /// reference previously handed out by [`Self::get()`] or [`Self::iter()`]:
    /// `av_dict_set()` moves the entry that used to sit in the new slot. Use
    /// [`Self::insert()`] to change a dictionary in place instead.
    pub fn set(mut self, key: &CStr, value: &CStr, flags: u32) -> Self {
        let mut dict = self.as_mut_ptr();
        // Only error on AVERROR_ENOMEM, so unwrap
        unsafe { ffi::av_dict_set(&mut dict, key.as_ptr(), value.as_ptr(), safe_flags(flags)) }
            .upgrade()
            .unwrap();
        self
    }

    /// Same as [`Self::set()`], with `value` stored as its decimal representation.
    pub fn set_int(mut self, key: &CStr, value: i64, flags: u32) -> Self {
        let mut dict = self.as_mut_ptr();
        // Only error on AVERROR_ENOMEM, so unwrap
        unsafe { ffi::av_dict_set_int(&mut dict, key.as_ptr(), value, safe_flags(flags)) }
            .upgrade()
            .unwrap();
        self
    }

    /// Store `key` in place, replacing the value it had.
    ///
    /// This is [`Self::set()`] without the ownership dance. Adding an entry
    /// invalidates the entry references handed out earlier, which the `&mut
    /// self` borrow is what keeps track of.
    pub fn insert(&mut self, key: &CStr, value: &CStr) {
        // `av_dict_set()` only reassigns the pointer it is handed when that
        // pointer is null, which an `AVDictionary` never is, so there is
        // nothing to write back afterwards.
        let mut dict = self.as_mut_ptr();
        // Only error on AVERROR_ENOMEM, so unwrap
        unsafe { ffi::av_dict_set(&mut dict, key.as_ptr(), value.as_ptr(), 0) }
            .upgrade()
            .unwrap();
    }

    /// Same as [`Self::insert()`], with `value` stored as its decimal
    /// representation.
    pub fn insert_int(&mut self, key: &CStr, value: i64) {
        let mut dict = self.as_mut_ptr();
        // Only error on AVERROR_ENOMEM, so unwrap
        unsafe { ffi::av_dict_set_int(&mut dict, key.as_ptr(), value, 0) }
            .upgrade()
            .unwrap();
    }

    /// Parse `str` on top of this dictionary, see [`Self::from_string()`].
    pub fn parse_string(
        mut self,
        str: &CStr,
        key_val_sep: &CStr,
        pairs_sep: &CStr,
        flags: u32,
    ) -> Result<Self> {
        let mut dict = self.as_mut_ptr();
        unsafe {
            ffi::av_dict_parse_string(
                &mut dict,
                str.as_ptr(),
                key_val_sep.as_ptr(),
                pairs_sep.as_ptr(),
                safe_flags(flags),
            )
        }
        .upgrade()?;
        Ok(self)
    }

    /// Merge every entry of `another` into this dictionary, replacing the ones
    /// whose keys it has in common.
    ///
    /// Takes `self` by value for the same reason [`Self::set()`] does.
    pub fn copy(mut self, another: &AVDictionary, flags: u32) -> Self {
        let mut dict = self.as_mut_ptr();
        // Only error on AVERROR_ENOMEM, so unwrap
        unsafe { ffi::av_dict_copy(&mut dict, another.as_ptr(), safe_flags(flags)) }
            .upgrade()
            .unwrap();
        self
    }

    /// Whether `key` is present.
    pub fn contains_key(&self, key: &CStr) -> bool {
        self.get(key, None, 0).is_some()
    }

    /// The value stored under `key`.
    ///
    /// Returns `None` when there is no such entry. Like every other lookup
    /// here, it ignores case (`AV_DICT_MATCH_CASE` is not set).
    pub fn get_value(&self, key: &CStr) -> Option<&CStr> {
        // Going through `AVDictionaryEntry::value()` would tie the result to a
        // local entry reference; the strings belong to the dictionary, so read
        // the entry directly and borrow them for `&self` instead.
        let entry = unsafe { ffi::av_dict_get(self.as_ptr(), key.as_ptr(), ptr::null(), 0) };
        let entry = unsafe { entry.as_ref() }?;
        if entry.value.is_null() {
            return None;
        }
        Some(unsafe { CStr::from_ptr(entry.value) })
    }

    /// The value stored under `key`, read as an integer.
    ///
    /// Returns `None` when there is no such entry, when the value is not valid
    /// UTF-8, or when it does not parse as an [`i64`].
    pub fn get_int(&self, key: &CStr) -> Option<i64> {
        self.get_value(key)?.to_str().ok()?.parse().ok()
    }

    /// The number of entries in this dictionary.
    pub fn len(&self) -> usize {
        // `av_dict_count()` hands back a `c_int` counting entries, so never negative.
        unsafe { ffi::av_dict_count(self.as_ptr()) as usize }
    }

    /// Whether this dictionary holds no entries.
    ///
    /// An [`AVDictionary`] cannot represent an empty dictionary — handing
    /// `av_dict_set()` a null value for the last entry frees the dictionary
    /// itself — so this is false for every dictionary that can be built today.
    /// It asks [`Self::len()`] rather than assuming, so it stays correct if that
    /// ever stops being true.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A duplicate of this dictionary, as a freshly allocated raw pointer owned
    /// by the caller.
    ///
    /// The caller becomes responsible for it: either hand it to an FFI call
    /// that takes it over, or release it with `av_dict_free()`.
    ///
    /// `av_dict_copy()` only fails on OOM, and an [`AVDictionary`] always holds
    /// at least one entry, so the result is never null.
    fn duplicate_raw(&self, flags: u32) -> Result<*mut ffi::AVDictionary> {
        let mut copied = ptr::null_mut();
        unsafe { ffi::av_dict_copy(&mut copied, self.as_ptr(), safe_flags(flags)) }.upgrade()?;
        Ok(copied)
    }

    /// Get dictionary entries as a string.
    ///
    /// Create a string containing dictionary's entries.
    /// Such string may be passed back to `Self::parse_string()`.
    pub fn get_string(&self, key_val_sep: u8, pairs_sep: u8) -> Result<CString> {
        let mut s = ptr::null_mut();
        unsafe { ffi::av_dict_get_string(self.as_ptr(), &mut s, key_val_sep as _, pairs_sep as _) }
            .upgrade()?;
        let result = unsafe { CStr::from_ptr(s).to_owned() };
        unsafe {
            ffi::av_freep(&mut s as *mut _ as *mut c_void);
        }
        Ok(result)
    }
}

/// Hand an FFI call a *copy* of `options`, then rebind `options` to whatever the
/// call left behind.
///
/// The FFmpeg entry points that take an `AVDictionary **` disagree on what they
/// do with it: `avformat_open_input()` frees it only on the success return,
/// `ffurl_open_whitelist()` (behind `avio_open2()`) frees it as soon as it has
/// applied it and can still fail afterwards, and `avfilter_init_dict()` frees it
/// whichever way it returns. Handing the caller's dictionary over directly
/// therefore leaves it dangling on at least one of those paths.
///
/// Giving the call a duplicate makes all of them behave alike: `options` is
/// always rebound to memory this side owns, whether the call succeeded or not.
/// That rebinding happens *before* the call's return value is checked, and the
/// ordering is the whole point — do not fold it into an early return.
pub(crate) fn with_copied_options<T>(
    options: &mut Option<AVDictionary>,
    call: impl FnOnce(&mut *mut ffi::AVDictionary) -> T,
) -> Result<T> {
    let mut options_ptr = match options.as_ref() {
        Some(dict) => dict.duplicate_raw(0)?,
        None => ptr::null_mut(),
    };

    let output = call(&mut options_ptr);

    // The dictionary the caller passed in was copied rather than handed over,
    // so it is still ours: rebinding `options` releases it here instead of
    // leaking it.
    *options = options_ptr
        .upgrade()
        .map(|x| unsafe { AVDictionary::from_raw(x) });

    Ok(output)
}

impl<'dict> AVDictionary {
    /// Get a dictionary entry with matching key.
    ///
    /// The returned entry key or value must not be changed, or it will
    /// cause undefined behavior.
    ///
    /// To iterate through all the dictionary entries, you can set the matching key
    /// to the null string "" and set the AV_DICT_IGNORE_SUFFIX flag.
    pub fn get(
        &'dict self,
        key: &CStr,
        prev: Option<AVDictionaryEntryRef>,
        flags: u32,
    ) -> Option<AVDictionaryEntryRef<'dict>> {
        let prev_ptr = match prev {
            Some(entry) => entry.as_ptr(),
            None => ptr::null(),
        };
        unsafe { ffi::av_dict_get(self.as_ptr(), key.as_ptr(), prev_ptr, flags as i32) }
            .upgrade()
            .map(|ptr| unsafe { AVDictionaryEntryRef::from_raw(ptr) })
    }

    /// Iterates through all entries in the dictionary by reference.
    pub fn iter(&'dict self) -> AVDictionaryIter<'dict> {
        AVDictionaryIter {
            dict: self,
            ptr: ptr::null(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl Clone for AVDictionary {
    /// Similar to `Self::copy()`, while set the copy flag to `0`.
    fn clone(&self) -> Self {
        let mut newer = ptr::null_mut();
        unsafe { ffi::av_dict_copy(&mut newer, self.as_ptr(), 0) }
            .upgrade()
            .unwrap();
        unsafe { Self::from_raw(NonNull::new(newer).unwrap()) }
    }
}

impl Drop for AVDictionary {
    fn drop(&mut self) {
        let mut dict = self.as_mut_ptr();
        unsafe { ffi::av_dict_free(&mut dict) }
    }
}

impl<'a> Extend<(&'a CStr, &'a CStr)> for AVDictionary {
    fn extend<T: IntoIterator<Item = (&'a CStr, &'a CStr)>>(&mut self, iter: T) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<'dict> IntoIterator for &'dict AVDictionary {
    type IntoIter = AVDictionaryIter<'dict>;
    type Item = AVDictionaryEntryRef<'dict>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Debug for AVDictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = f.debug_map();

        for entry in self.into_iter() {
            out.entry(&entry.key(), &entry.value());
        }

        out.finish()
    }
}

/// Iterator over [`AVDictionary`] by reference.
pub struct AVDictionaryIter<'dict> {
    dict: &'dict AVDictionary,
    ptr: *const ffi::AVDictionaryEntry,
    _phantom: std::marker::PhantomData<&'dict ()>,
}

impl<'dict> Iterator for AVDictionaryIter<'dict> {
    type Item = AVDictionaryEntryRef<'dict>;
    fn next(&mut self) -> Option<Self::Item> {
        self.ptr = unsafe { ffi::av_dict_iterate(self.dict.as_ptr(), self.ptr) };
        self.ptr
            .upgrade()
            .map(|x| unsafe { AVDictionaryEntryRef::from_raw(x) })
    }
}

wrap_ref_mut!(AVDictionaryEntry: ffi::AVDictionaryEntry);

impl AVDictionaryEntry {
    /// The key of this entry, borrowed for as long as the dictionary it comes
    /// from is not modified.
    pub fn key(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.key) }
    }

    /// The value of this entry, borrowed for as long as the dictionary it comes
    /// from is not modified.
    pub fn value(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.value) }
    }
}

#[cfg(test)]
mod test {
    use super::AVDictionary;

    #[test]
    fn set() {
        let dict = AVDictionary::new(c"bob", c"alice", 0);
        assert_eq!(dict.get_value(c"bob").unwrap(), c"alice");
        assert_eq!(dict.len(), 1);

        // `set()` adds, it does not disturb the entries around it.
        let dict = dict.set(c"foo", c"bar", 0);
        assert_eq!(dict.len(), 2);
        assert_eq!(dict.get_value(c"foo").unwrap(), c"bar");
        assert_eq!(dict.get_value(c"bob").unwrap(), c"alice");

        // Setting a key that is already there replaces its value rather than
        // adding a second entry with the same key.
        let dict = dict.set(c"bob", c"carol", 0);
        assert_eq!(dict.len(), 2);
        assert_eq!(dict.get_value(c"bob").unwrap(), c"carol");
    }

    #[test]
    fn set_int() {
        let dict = AVDictionary::new_int(c"bob", 2233, 0).set_int(c"foo", 123456789123456789, 0);
        assert_eq!(
            c"123456789123456789",
            dict.get(c"foo", None, 0).unwrap().value()
        );
    }

    #[test]
    fn get() {
        let dict = AVDictionary::new(c"bob", c"alice", 0);
        assert_eq!(c"alice", dict.get(c"bob", None, 0).unwrap().value());

        let dict = AVDictionary::new(c"bob", c"alice", 0)
            .set(c"bob", c"alice", 0)
            .set(c"bob", c"alice", 0)
            .set(c"bob", c"alice", 0)
            .set(c"bob", c"alice", 0);
        assert_eq!(c"alice", dict.get(c"bob", None, 0).unwrap().value());

        let dict = AVDictionary::new(c"foo", c"bar", 0).set(c"bob", c"alice", 0);
        assert_eq!(c"bar", dict.get(c"foo", None, 0).unwrap().value());
        assert_eq!(c"alice", dict.get(c"bob", None, 0).unwrap().value());

        // Find `foo` after after entry of `bob` will fail.
        let entry = dict.get(c"bob", None, 0).unwrap();
        assert_eq!(c"alice", entry.value());
        assert!(dict.get(c"foo", Some(entry), 0).is_none());

        // Shadowing.
        let dict = AVDictionary::new(c"bob", c"alice0", 0)
            .set(c"bob", c"alice1", 0)
            .set(c"bob", c"alice2", 0)
            .set(c"bob", c"alice3", 0)
            .set(c"bob", c"alice4", 0);

        let entry = dict.get(c"bob", None, 0).unwrap();
        assert_eq!(c"alice4", entry.value());
        assert_eq!(c"bob", entry.key());
        assert!(dict.get(c"bob", Some(entry), 0).is_none());
    }

    #[test]
    fn copy() {
        let dicta = AVDictionary::new(c"a", c"b", 0).set(c"c", c"d", 0);

        let dictc = dicta.clone();
        assert_eq!(c"a:b-c:d", dictc.get_string(b':', b'-').unwrap().as_c_str());

        let dictb = AVDictionary::new(c"foo", c"bar", 0)
            .set(c"alice", c"bob", 0)
            .copy(&dictc, 0);
        assert_eq!(
            c"foo:bar-alice:bob-a:b-c:d",
            dictb.get_string(b':', b'-').unwrap().as_c_str(),
        );

        let dicta = dicta.set(c"e", c"f", 0);

        assert_eq!(c"b", dicta.get(c"a", None, 0).unwrap().value());
        assert_eq!(c"d", dicta.get(c"c", None, 0).unwrap().value());
        assert_eq!(c"f", dicta.get(c"e", None, 0).unwrap().value());

        assert_eq!(c"b", dictb.get(c"a", None, 0).unwrap().value());
        assert_eq!(c"d", dictb.get(c"c", None, 0).unwrap().value());
        assert!(dictb.get(c"e", None, 0).is_none());
    }

    #[test]
    fn serialization() {
        let dict = AVDictionary::new(c"a", c"b", 0)
            .set(c"c", c"d", 0)
            .set(c"foo", c"bar", 0)
            .set(c"bob", c"alice", 0);
        assert_eq!(
            c"a:b-c:d-foo:bar-bob:alice",
            dict.get_string(b':', b'-').unwrap().as_c_str()
        );
        let dict = dict.set(c"rust", c"c", 0);
        assert_eq!(
            c"a:b-c:d-foo:bar-bob:alice-rust:c",
            dict.get_string(b':', b'-').unwrap().as_c_str()
        );
    }

    #[test]
    fn deserialization() {
        let dict =
            AVDictionary::from_string(c"a:b-c:d-foo:bar-bob:alice-rust:c", c":", c"-", 0).unwrap();
        assert_eq!(
            c"a:b-c:d-foo:bar-bob:alice-rust:c",
            dict.get_string(b':', b'-').unwrap().as_c_str()
        );
    }

    /// The `AV_DICT_DONT_STRDUP_*` flags hand the ownership of the key and value
    /// pointers to the dictionary. A `&CStr` is only borrowed for the call, so
    /// honouring them would let the dictionary `av_free()` memory its caller
    /// still owns — the earlier behaviour was a use-after-free followed by a
    /// double free.
    #[test]
    fn ownership_flags_are_ignored() {
        let key = std::ffi::CString::new("k").unwrap();
        let value = std::ffi::CString::new("v").unwrap();
        let flags = crate::ffi::AV_DICT_DONT_STRDUP_KEY | crate::ffi::AV_DICT_DONT_STRDUP_VAL;

        // Whatever happens to the borrowed strings afterwards, the dictionary
        // holds its own copies.
        let dict = AVDictionary::new(&key, &value, flags);
        drop(key);
        drop(value);

        let mut dict = dict.set(c"k2", c"v2", flags);
        assert_eq!(
            c"k:v-k2:v2",
            dict.get_string(b':', b'-').unwrap().as_c_str()
        );

        let other = AVDictionary::new(c"k3", c"v3", 0);
        dict = dict.copy(&other, flags);
        assert_eq!(
            c"k:v-k2:v2-k3:v3",
            dict.get_string(b':', b'-').unwrap().as_c_str()
        );
    }

    /// A string holding no pairs parses successfully without producing a
    /// dictionary, which the `Option` return type covers; it used to panic.
    #[test]
    fn from_string_without_pairs_is_none() {
        assert!(AVDictionary::from_string(c"", c":", c"-", 0).is_none());
    }

    /// `insert()` is the in-place counterpart of `set()`, so it can be used
    /// where the dictionary has to stay in a `&mut` binding.
    #[test]
    fn insert_in_place() {
        let mut dict = AVDictionary::new(c"a", c"1", 0);
        dict.insert(c"b", c"2");
        dict.insert(c"a", c"3");
        dict.insert_int(c"n", -7);

        assert_eq!(dict.len(), 3);
        assert_eq!(dict.get_value(c"a").unwrap(), c"3");
        assert_eq!(dict.get_value(c"b").unwrap(), c"2");
        assert_eq!(dict.get_int(c"n"), Some(-7));
    }

    /// Lookups have to report a miss rather than a default.
    #[test]
    fn lookups_that_miss() {
        let mut dict = AVDictionary::new(c"a", c"1", 0);
        dict.insert(c"not_a_number", c"alice");

        assert!(dict.contains_key(c"a"));
        assert!(!dict.contains_key(c"missing"));
        assert!(dict.get_value(c"missing").is_none());

        // A value that is there but is not a number is a miss too.
        assert_eq!(dict.get_int(c"not_a_number"), None);
        assert_eq!(dict.get_int(c"missing"), None);
    }

    /// `Extend`, `iter` and the `Debug` rendering, which had no coverage.
    #[test]
    fn extend_iterate_and_debug() {
        let mut dict = AVDictionary::new(c"first", c"1", 0);
        dict.extend([(c"second", c"2"), (c"third", c"3")]);
        assert_eq!(dict.len(), 3);

        // Insertion order is preserved as long as no key is overwritten.
        let rendered = dict
            .iter()
            .map(|entry| {
                format!(
                    "{}={}",
                    entry.key().to_string_lossy(),
                    entry.value().to_string_lossy()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(rendered, "first=1,second=2,third=3");
        assert_eq!((&dict).into_iter().count(), dict.len());

        let debug = format!("{dict:?}");
        assert!(debug.contains(r#""first": "1""#), "{debug}");
        assert!(debug.contains(r#""third": "3""#), "{debug}");
    }

    /// `parse_string()` merges into a dictionary that already exists, unlike
    /// `from_string()`.
    #[test]
    fn parse_string_merges() {
        let dict = AVDictionary::new(c"a", c"1", 0)
            .parse_string(c"b:2-c:3", c":", c"-", 0)
            .unwrap();

        assert_eq!(dict.len(), 3);
        assert_eq!(dict.get_value(c"a").unwrap(), c"1");
        assert_eq!(dict.get_value(c"c").unwrap(), c"3");
    }
}
