//! The child inherits user settings, but terminal capabilities describe BKS Term.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
use windows::Win32::System::Environment::{FreeEnvironmentStringsW, GetEnvironmentStringsW};

const CAPABILITIES: [(&str, &str); 3] = [
    ("TERM", "xterm-256color"),
    ("COLORTERM", "truecolor"),
    ("TERM_PROGRAM", "bks-term"),
];

/// Capture the native block rather than vars_os, which omits hidden `=C:` drive
/// entries. Never mutate the parent environment or decode inherited UTF-16.
pub(super) fn capture(overrides: &BTreeMap<String, String>) -> io::Result<Vec<u16>> {
    validate_overrides(overrides)?;
    struct NativeBlock(PWSTR);

    impl Drop for NativeBlock {
        fn drop(&mut self) {
            // SAFETY: this pointer is the block returned by GetEnvironmentStringsW.
            unsafe {
                let _ = FreeEnvironmentStringsW(PCWSTR(self.0 .0));
            }
        }
    }

    // SAFETY: the API returns an independent, double-NUL-terminated snapshot.
    let pointer = unsafe { GetEnvironmentStringsW() };
    if pointer.is_null() {
        return Err(io::Error::last_os_error());
    }
    let native = NativeBlock(pointer);
    let mut entries = Vec::new();
    // SAFETY: the owned snapshot stays valid until native is dropped. Each
    // entry ends at a NUL and an empty entry terminates the block.
    unsafe {
        let mut cursor = native.0 .0;
        while *cursor != 0 {
            let start = cursor;
            while *cursor != 0 {
                cursor = cursor.add(1);
            }
            let length = cursor.offset_from(start) as usize;
            entries.push(std::slice::from_raw_parts(start, length).to_vec());
            cursor = cursor.add(1);
        }
    }
    Ok(apply_overrides(terminal_block(entries), overrides))
}

fn validate_overrides(overrides: &BTreeMap<String, String>) -> io::Result<()> {
    let mut names: Vec<Vec<u16>> = Vec::with_capacity(overrides.len());
    for (key, value) in overrides {
        let name: Vec<u16> = key.encode_utf16().collect();
        if key.is_empty()
            || key.contains(['=', '\0'])
            || value.contains('\0')
            || names
                .iter()
                .any(|known| compare_names(known, &name) == Ordering::Equal)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid or duplicate profile environment name/value",
            ));
        }
        names.push(name);
    }
    Ok(())
}

fn apply_overrides(block: Vec<u16>, overrides: &BTreeMap<String, String>) -> Vec<u16> {
    if overrides.is_empty() {
        return block;
    }
    let mut entries: Vec<Vec<u16>> = block[..block.len() - 1]
        .split(|unit| *unit == 0)
        .filter(|entry| !entry.is_empty())
        .map(<[u16]>::to_vec)
        .collect();
    for (key, value) in overrides {
        let key_wide: Vec<u16> = key.encode_utf16().collect();
        entries.retain(|entry| compare_names(name(entry), &key_wide) != Ordering::Equal);
        entries.push(format!("{key}={value}").encode_utf16().collect());
    }
    entries.sort_by(|first, second| compare_names(name(first), name(second)));
    let mut result = Vec::new();
    for entry in entries {
        result.extend(entry);
        result.push(0);
    }
    result.push(0);
    result
}

fn name(entry: &[u16]) -> &[u16] {
    // A drive entry is `=C:=C:\path`: its name includes the leading equals.
    let skip = usize::from(entry.first() == Some(&u16::from(b'=')));
    let end = entry[skip..]
        .iter()
        .position(|unit| *unit == u16::from(b'='))
        .map_or(entry.len(), |position| position + skip);
    &entry[..end]
}

fn compare_names(first: &[u16], second: &[u16]) -> Ordering {
    // SAFETY: environment names are nonempty UTF-16 slices with valid lengths.
    // Windows ordinal comparison preserves its case-insensitive lookup order,
    // independently of the user's locale and without lossy Unicode conversion.
    let result = unsafe { CompareStringOrdinal(first, second, true) };
    debug_assert_ne!(result.0, 0);
    result.0.cmp(&CSTR_EQUAL.0)
}

fn terminal_block(mut entries: Vec<Vec<u16>>) -> Vec<u16> {
    let owned_names: Vec<Vec<u16>> = CAPABILITIES
        .iter()
        .map(|(key, _)| *key)
        .chain(Some("TERM_PROGRAM_VERSION"))
        .map(|key| key.encode_utf16().collect())
        .collect();
    entries.retain(|entry| {
        !owned_names
            .iter()
            .any(|owned| compare_names(name(entry), owned) == Ordering::Equal)
    });
    for (key, value) in CAPABILITIES {
        entries.push(format!("{key}={value}").encode_utf16().collect());
    }
    entries.sort_by(|first, second| compare_names(name(first), name(second)));

    let mut block = Vec::new();
    for entry in entries {
        block.extend(entry);
        block.push(0);
    }
    // At least the three capability entries exist, so this is the second NUL.
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(block: &[u16]) -> Vec<Vec<u16>> {
        assert_eq!(&block[block.len() - 2..], &[0, 0]);
        assert_ne!(block[block.len() - 3], 0);
        block[..block.len() - 1]
            .split(|unit| *unit == 0)
            .filter(|entry| !entry.is_empty())
            .map(<[u16]>::to_vec)
            .collect()
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn capabilities_replace_all_host_spellings_and_remove_the_host_version() {
        let block = terminal_block(
            [
                "term=dumb",
                "Term=old-terminal",
                "CoLoRtErM=host-color",
                "tErM_pRoGrAm=vscode",
                "TeRm_PrOgRaM_vErSiOn=host-version",
                "NO_COLOR=1",
                "FORCE_COLOR=0",
                "CLICOLOR=0",
                "CLICOLOR_FORCE=1",
            ]
            .map(wide)
            .into(),
        );
        let expected = [
            "CLICOLOR=0",
            "CLICOLOR_FORCE=1",
            "COLORTERM=truecolor",
            "FORCE_COLOR=0",
            "NO_COLOR=1",
            "TERM=xterm-256color",
            "TERM_PROGRAM=bks-term",
        ]
        .map(wide);
        assert_eq!(entries(&block), expected);
    }

    #[test]
    fn inheritance_preserves_drive_aliases_empty_values_and_raw_utf16() {
        let mut raw_value = wide("RAW=");
        raw_value.extend([0xd800, u16::from(b'=')]);
        let inherited = [
            wide(r"=C:=C:\work Ж🙂"),
            wide(r"=D:=D:\other"),
            wide(r"Path=C:\custom;D:\tools"),
            wide("SSH_AUTH_SOCK=pipe=fixture"),
            wide("USER_FIXTURE=Ж🙂=keep"),
            wide("EMPTY="),
            raw_value,
        ];
        let result = entries(&terminal_block(inherited.to_vec()));
        for entry in inherited {
            assert!(result.contains(&entry));
        }
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn block_uses_windows_ordinal_order_and_exactly_two_final_nuls() {
        let result = entries(&terminal_block(
            [
                "é=value",
                "z=value",
                "Ä=value",
                "a=value",
                "_=value",
                r"=C:=C:\work",
            ]
            .map(wide)
            .into(),
        ));
        assert_eq!(
            result,
            [
                r"=C:=C:\work",
                "a=value",
                "COLORTERM=truecolor",
                "TERM=xterm-256color",
                "TERM_PROGRAM=bks-term",
                "z=value",
                "_=value",
                "Ä=value",
                "é=value",
            ]
            .map(wide)
        );
        assert_eq!(entries(&terminal_block(Vec::new())).len(), 3);
    }

    #[test]
    fn profile_values_replace_case_insensitively_and_preserve_other_native_entries() {
        let inherited =
            terminal_block([wide("Path=parent"), wide(r"=C:=C:\raw"), wide("KEEP=Ж🙂")].into());
        let overrides = BTreeMap::from([
            ("PATH".into(), "child".into()),
            ("EMPTY".into(), String::new()),
            ("TERM".into(), "dumb".into()),
            ("BKS_FIXTURE".into(), "a=b Ж🙂".into()),
        ]);
        validate_overrides(&overrides).unwrap();
        let result = entries(&apply_overrides(inherited, &overrides));
        for value in [
            "PATH=child",
            "EMPTY=",
            "TERM=dumb",
            "BKS_FIXTURE=a=b Ж🙂",
            r"=C:=C:\raw",
            "KEEP=Ж🙂",
        ] {
            assert!(result.contains(&wide(value)));
        }
        assert!(!result.contains(&wide("Path=parent")));
    }

    #[test]
    fn profile_environment_rejects_nuls_empty_names_and_windows_duplicate_spellings() {
        for overrides in [
            BTreeMap::from([(String::new(), "fixture".into())]),
            BTreeMap::from([("BAD=NAME".into(), "fixture".into())]),
            BTreeMap::from([("BAD\0NAME".into(), "fixture".into())]),
            BTreeMap::from([("GOOD".into(), "bad\0value".into())]),
            BTreeMap::from([
                ("Path".into(), "first".into()),
                ("PATH".into(), "second".into()),
            ]),
        ] {
            assert_eq!(
                validate_overrides(&overrides).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
