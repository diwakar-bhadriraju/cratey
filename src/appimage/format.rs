//! AppImage format detection.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::{AppError, Result};

/// The detected AppImage container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// ELF-wrapped squashfs (runtime + squashfs payload appended).
    Type2Elf,
    /// Old ISO9660/zisofs based.
    Type1Iso,
}

/// Reads the first 4 bytes of a file.
fn magic(path: &Path) -> Result<[u8; 4]> {
    let mut f = File::open(path)?;
    let mut m = [0u8; 4];
    f.read_exact(&mut m)?;
    Ok(m)
}

/// Detects the AppImage type from the file magic, or rejects the file.
pub fn detect(path: &Path) -> Result<Format> {
    let m = match magic(path) {
        Err(e) => return Err(AppError::AppImage(format!("{path:?}: {e}"))),
        Ok(m) => m,
    };
    if m == *b"\x7fELF" {
        Ok(Format::Type2Elf)
    } else if m == [0x50, 0x4b, 0x03, 0x04] {
        Err(AppError::AppImage(
            "Type-1 ISO AppImages are not supported by this build; only Type-2 ELF AppImages are supported"
                .to_string(),
        ))
    } else {
        Err(AppError::AppImage("Not an AppImage".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appimage::testutil;

    #[test]
    fn detects_elf_type2() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.AppImage");
        std::fs::write(&p, testutil::synthetic_elf(true)).unwrap();
        assert_eq!(detect(&p).unwrap(), Format::Type2Elf);
    }

    #[test]
    fn rejects_zip_magic_as_type1() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("b.AppImage");
        std::fs::write(&p, testutil::zip_magic_only()).unwrap();
        let err = detect(&p).unwrap_err();
        assert!(matches!(err, AppError::AppImage(_)));
        assert!(err.to_string().contains("Type-1"));
    }

    #[test]
    fn rejects_non_appimage() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("c.bin");
        std::fs::write(&p, testutil::not_an_appimage()).unwrap();
        assert!(matches!(detect(&p), Err(AppError::AppImage(_))));
    }
}
