#!/usr/bin/env bash
# make_fixture.sh — synthesizes a minimal Type-2 ELF "AppImage" for testing.
#
# The file is 4096 bytes: a valid-looking ELF64 header (so `format::detect`
# returns Type2Elf) with a `.shstrtab` containing "squashfs" and `hsqs` magic
# at the claimed squashfs offset (so `elf_offset::find` succeeds). It is NOT a
# runnable AppImage and does not contain a real squashfs payload — v1's
# inspector only needs the header.
#
# Usage: tools/make_fixture.sh [output]
#   default output: ./fixture.AppImage

set -euo pipefail

OUT="${1:-fixture.AppImage}"

# 4096 zeroed bytes.
head -c 4096 /dev/zero > "$OUT"

# ELF magic + class (2 = 64-bit) + data (1 = LE).
printf '\x7fELF' | dd of="$OUT" bs=1 seek=0 conv=notrunc status=none
printf '\x02'   | dd of="$OUT" bs=1 seek=4  conv=notrunc status=none
printf '\x01'   | dd of="$OUT" bs=1 seek=5  conv=notrunc status=none
# e_type = ET_EXEC (2), e_machine = EM_X86_64 (62), e_ehsize = 64.
printf '\x02\x00'         | dd of="$OUT" bs=1 seek=16 conv=notrunc status=none
printf '\x3e\x00'         | dd of="$OUT" bs=1 seek=18 conv=notrunc status=none
printf '\x40\x00'         | dd of="$OUT" bs=1 seek=52 conv=notrunc status=none
# e_shoff = 512 (LE u64).
printf '\x00\x02\x00\x00\x00\x00\x00\x00' | dd of="$OUT" bs=1 seek=40 conv=notrunc status=none
# e_shnum = 3, e_shstrndx = 1.
printf '\x03\x00' | dd of="$OUT" bs=1 seek=60 conv=notrunc status=none
printf '\x01\x00' | dd of="$OUT" bs=1 seek=62 conv=notrunc status=none

# .shstrtab at offset 256: "\0.shstrtab\0squashfs\0"
printf '\x00.shstrtab\x00squashfs\x00' | dd of="$OUT" bs=1 seek=256 conv=notrunc status=none

# Section header [1] = .shstrtab (name=0, type=3 SHT_STRTAB, offset=256, size=N)
SH1=$((512 + 64))
printf '\x00\x00\x00\x00'                                     | dd of="$OUT" bs=1 seek=$SH1      conv=notrunc status=none
printf '\x03\x00\x00\x00'                                     | dd of="$OUT" bs=1 seek=$((SH1+4))  conv=notrunc status=none
printf '\x00\x01\x00\x00\x00\x00\x00\x00'                     | dd of="$OUT" bs=1 seek=$((SH1+24)) conv=notrunc status=none
printf '\x0f\x00\x00\x00\x00\x00\x00\x00'                     | dd of="$OUT" bs=1 seek=$((SH1+32)) conv=notrunc status=none

# Section header [2] = squashfs (name=10, type=1 SHT_PROGBITS, offset=3072, size=256)
SH2=$((512 + 128))
printf '\x0a\x00\x00\x00'       | dd of="$OUT" bs=1 seek=$SH2     conv=notrunc status=none
printf '\x01\x00\x00\x00'       | dd of="$OUT" bs=1 seek=$((SH2+4)) conv=notrunc status=none
printf '\x00\x0c\x00\x00\x00\x00\x00\x00' | dd of="$OUT" bs=1 seek=$((SH2+24)) conv=notrunc status=none
printf '\x00\x01\x00\x00\x00\x00\x00\x00' | dd of="$OUT" bs=1 seek=$((SH2+32)) conv=notrunc status=none

# hsqs magic (LE) at the squashfs offset 3072.
printf '\x68\x73\x71\x73' | dd of="$OUT" bs=1 seek=3072 conv=notrunc status=none

echo "wrote $OUT ($(wc -c < "$OUT") bytes)"