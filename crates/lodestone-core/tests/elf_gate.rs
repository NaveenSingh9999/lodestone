use std::path::Path;

use lodestone_core::runtime::check_elf_alignment;
use lodestone_core::runtime::device_page_size;
use lodestone_core::runtime::elf::ElfGate;

fn elf64_with_load_segments(segments: &[(u64, u64)]) -> Vec<u8> {
    let n = segments.len();
    let mut buf = vec![0u8; 64 + 56 * n];
    buf[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    buf[4] = 2;
    buf[5] = 1;
    buf[0x20..0x28].copy_from_slice(&(64u64).to_le_bytes());
    buf[0x36..0x38].copy_from_slice(&(56u16).to_le_bytes());
    buf[0x38..0x3A].copy_from_slice(&(n as u16).to_le_bytes());
    for (i, (align, memsz)) in segments.iter().enumerate() {
        let b = 64 + 56 * i;
        buf[b..b + 4].copy_from_slice(&1u32.to_le_bytes());
        buf[b + 40..b + 48].copy_from_slice(&memsz.to_le_bytes());
        buf[b + 48..b + 56].copy_from_slice(&align.to_le_bytes());
    }
    buf
}

fn elf64_with_load_aligns(aligns: &[u64]) -> Vec<u8> {
    elf64_with_load_segments(&aligns.iter().map(|a| (*a, 4096u64)).collect::<Vec<_>>())
}

#[test]
fn four_k_aligned_library_passes_on_four_k_devices_only() {
    let elf = elf64_with_load_aligns(&[4096]);
    assert_eq!(check_elf_alignment(&elf, 4096), ElfGate::Pass);
    assert_eq!(
        check_elf_alignment(&elf, 16384),
        ElfGate::Fail {
            required_bytes: 16384,
            worst_align: 4096
        }
    );
}

#[test]
fn sixteen_k_aligned_library_passes_everywhere() {
    let elf = elf64_with_load_aligns(&[16384]);
    assert_eq!(check_elf_alignment(&elf, 4096), ElfGate::Pass);
    assert_eq!(check_elf_alignment(&elf, 16384), ElfGate::Pass);
}

#[test]
fn zero_alignment_counts_as_unaligned() {
    let elf = elf64_with_load_aligns(&[16384, 0]);
    assert_eq!(
        check_elf_alignment(&elf, 16384),
        ElfGate::Fail {
            required_bytes: 16384,
            worst_align: 0
        }
    );
}

#[test]
fn malformed_inputs_are_invalid() {
    assert_eq!(check_elf_alignment(&[], 4096), ElfGate::Invalid);
    assert_eq!(
        check_elf_alignment(b"not an elf at all....", 4096),
        ElfGate::Invalid
    );
    assert_eq!(
        check_elf_alignment(&elf64_with_load_aligns(&[4096]), 0),
        ElfGate::Invalid
    );

    let mut elf = elf64_with_load_aligns(&[16384]);
    elf[4] = 1;
    assert_eq!(check_elf_alignment(&elf, 4096), ElfGate::Invalid);
}

#[test]
fn load_segments_without_size_are_ignored() {
    let elf = elf64_with_load_segments(&[(0, 0), (4096, 4096)]);
    assert_eq!(
        check_elf_alignment(&elf, 16384),
        ElfGate::Fail {
            required_bytes: 16384,
            worst_align: 4096
        }
    );
    let elf = elf64_with_load_segments(&[(4096, 0), (16384, 4096)]);
    assert_eq!(check_elf_alignment(&elf, 16384), ElfGate::Pass);
}

#[test]
fn real_system_libraries_parse() {
    let candidates = [
        "/system/lib64/libc.so",
        "/system/usr/lib64/libc.so",
        "/data/data/com.termux/files/usr/lib/libc.so",
    ];
    let found = candidates.iter().find(|p| Path::new(p).exists());
    if found.is_none() {
        return;
    }
    let path = found.unwrap();
    let bytes = std::fs::read(path).unwrap();
    let gate4096 = check_elf_alignment(&bytes, 4096);
    assert_ne!(gate4096, ElfGate::Invalid, "{path} must parse");
    if let Some(page) = device_page_size() {
        let gate_real = check_elf_alignment(&bytes, page);
        assert!(
            gate_real == ElfGate::Pass || matches!(gate_real, ElfGate::Fail { .. }),
            "{path} must gate consistently against real page size {page}"
        );
    }
}

#[test]
fn device_page_size_is_sane() {
    if let Some(page) = device_page_size() {
        assert!(page.is_power_of_two());
        assert!(
            page == 4096 || page == 16384 || page == 65536,
            "unexpected page {page}"
        );
    }
}
