//! AppImage container inspection.
//!
//! AppImages are never executed and never mounted. Detection is header-only;
//! v1 does not require `unsquashfs`.

pub mod elf_offset;
pub mod format;
pub mod icon;
pub mod scan;
pub mod update_info;

#[cfg(test)]
pub(crate) mod testutil {
    //! Helpers to build synthetic ELF fixtures for tests.

    /// Builds a minimal 64-bit ELF header with a `.shstrtab`-backed
    /// `squashfs` section whose offset is padded with `hsqs` magic.
    ///
    /// Layout (all little-endian):
    /// - ELF64 header (64 bytes)
    /// - `.shstrtab` string table containing `squashfs`
    /// - padding
    /// - `hsqs` superblock bytes at `squashfs.offset`
    pub fn synthetic_elf(with_squashfs: bool) -> Vec<u8> {
        let mut buf = vec![0u8; 4096];
        // e_ident
        buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        buf[4] = 2; // EI_CLASS: ELFCLASS64
        buf[5] = 1; // EI_DATA: little-endian
                    // e_type = ET_EXEC, e_machine = EM_X86_64
        buf[16..18].copy_from_slice(&2u16.to_le_bytes());
        buf[18..20].copy_from_slice(&62u16.to_le_bytes());
        // e_ehsize = 64
        buf[52..54].copy_from_slice(&64u16.to_le_bytes());

        let shstr_offset = 256usize;
        let mut shstr = vec![0u8];
        shstr.extend_from_slice(b".shstrtab\0");
        // Place the "squashfs" name inside .shstrtab (offset 10 → name value 10).
        shstr.extend_from_slice(b"squashfs\0");
        buf[shstr_offset..shstr_offset + shstr.len()].copy_from_slice(&shstr);

        // Section header table starts after the payload at a fixed offset.
        let shoff = 512usize;
        let strtab_ndx: u32 = 1;
        let squashfs_ndx: u32 = 2;

        // Section header 0: null.
        let sh = |buf: &mut [u8], sh_ndx: usize, name: u32, ty: u32, offset: usize, size: usize| {
            let base = shoff + sh_ndx * 64;
            buf[base..base + 4].copy_from_slice(&name.to_le_bytes());
            buf[base + 4..base + 8].copy_from_slice(&ty.to_le_bytes());
            // sh_offset at +24 (8 bytes), sh_size at +32 (8 bytes), sh_addralign at +56.
            buf[base + 24..base + 32].copy_from_slice(&(offset as u64).to_le_bytes());
            buf[base + 32..base + 40].copy_from_slice(&(size as u64).to_le_bytes());
            buf[base + 56..base + 64].copy_from_slice(&8u64.to_le_bytes());
        };

        // .shstrtab section header (name 1: ".shstrtab\0" name offset 0).
        let shstr_size = shstr.len();
        sh(
            &mut buf,
            strtab_ndx as usize,
            0,
            3,
            shstr_offset,
            shstr_size,
        );

        // squashfs section header: name value 10 ("squashfs"), SHT_PROGBITS (1).
        let squashfs_offset = 3072usize;
        if with_squashfs {
            sh(&mut buf, squashfs_ndx as usize, 10, 1, squashfs_offset, 256);
            // hsqs little-endian magic at claimed offset (0x73717368 LE).
            buf[squashfs_offset..squashfs_offset + 4].copy_from_slice(&0x73717368u32.to_le_bytes());
            buf[squashfs_offset + 16..squashfs_offset + 24].copy_from_slice(&4u64.to_le_bytes());
            // magic
        }

        // e_shoff at +40, e_shnum at +60, e_shstrndx at +62.
        buf[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
        buf[60..62].copy_from_slice(&3u16.to_le_bytes()); // 3 section headers
        buf[62..64].copy_from_slice(&(strtab_ndx as u16).to_le_bytes());
        buf
    }

    /// A type-1 AppImage signature: ZIP local file header magic.
    pub fn zip_magic_only() -> Vec<u8> {
        let mut buf = vec![0u8; 128];
        buf[0..4].copy_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        buf
    }

    pub fn not_an_appimage() -> Vec<u8> {
        vec![0x00, 0x01, 0x02, 0x03, 0x04]
    }
}
