//! Locate the squashfs payload offset in a Type-2 ELF AppImage.

use std::path::Path;

use goblin::elf::Elf;

use crate::error::{AppError, Result};

/// Offset of the squashfs payload within the file.
pub type SquashfsOffset = u64;

/// `hsqs` magic, little-endian (squashfs superblock signature).
pub const HSQS: u32 = 0x73717368;

/// Finds the squashfs payload offset: prefers the `squashfs` ELF section
/// (`sh_offset + sh_addr`), falling back to scanning for `hsqs` magic.
pub fn find(path: &Path) -> Result<SquashfsOffset> {
    let data = std::fs::read(path)?;
    find_from_bytes(&data, path)
}

fn find_from_bytes(data: &[u8], path: &Path) -> Result<SquashfsOffset> {
    // Try the ELF section first.
    if let Some(off) = section_offset(data) {
        if has_hsqs(data, off) {
            return Ok(off);
        }
    }
    // Fall back to a direct magic scan.
    if let Some(i) = scan_hsqs(data) {
        return Ok(i as u64);
    }
    Err(AppError::AppImage(format!(
        "{path:?}: not a valid AppImage: squashfs not found"
    )))
}

/// Returns `sh_offset + sh_addr` for a section named `squashfs`, if present.
fn section_offset(data: &[u8]) -> Option<SquashfsOffset> {
    let elf = Elf::parse(data).ok()?;
    for sh in &elf.section_headers {
        if let Some(name) = elf.shdr_strtab.get_at(sh.sh_name) {
            if name == "squashfs" {
                return Some(sh.sh_offset + sh.sh_addr);
            }
        }
    }
    None
}

fn has_hsqs(data: &[u8], off: SquashfsOffset) -> bool {
    let start = off as usize;
    data.get(start..start + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == HSQS)
        .unwrap_or(false)
}

fn scan_hsqs(data: &[u8]) -> Option<usize> {
    let magic = HSQS.to_le_bytes();
    data.windows(4).position(|w| w == magic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appimage::testutil;

    #[test]
    fn finds_squashfs_section() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.AppImage");
        std::fs::write(&p, testutil::synthetic_elf(true)).unwrap();
        assert_eq!(find(&p).unwrap(), 3072);
    }

    #[test]
    fn falls_back_to_magic_scan_without_section() {
        // Without a squashfs section header, still find the hsqs bytes.
        let mut data = testutil::synthetic_elf(false);
        // Inject hsqs somewhere not referenced by any section.
        data[4000..4004].copy_from_slice(&HSQS.to_le_bytes());
        assert_eq!(
            find_from_bytes(&data, std::path::Path::new("/x")).unwrap(),
            4000
        );
    }

    #[test]
    fn errors_when_no_squashfs() {
        let mut data = testutil::synthetic_elf(false);
        data.truncate(256);
        let err = find_from_bytes(&data, std::path::Path::new("/y")).unwrap_err();
        assert!(err.to_string().contains("squashfs not found"));
    }
}
