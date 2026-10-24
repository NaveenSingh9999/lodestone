#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfGate {
    Pass,
    Fail {
        required_bytes: u32,
        worst_align: u64,
    },
    Invalid,
}

const PT_LOAD: u32 = 1;
const MIN_PHENTSIZE: usize = 56;

pub fn check_elf_alignment(elf: &[u8], page_size: u32) -> ElfGate {
    parse_gate(elf, page_size).unwrap_or(ElfGate::Invalid)
}

fn parse_gate(elf: &[u8], page_size: u32) -> Option<ElfGate> {
    if page_size == 0 || !page_size.is_power_of_two() {
        return None;
    }
    if elf.len() < 64 || elf[0..4] != [0x7F, b'E', b'L', b'F'] {
        return None;
    }
    if elf[4] != 2 || elf[5] != 1 {
        return None;
    }

    let read_u64 = |off: usize| -> Option<u64> {
        let s = elf.get(off..off + 8)?;
        Some(u64::from_le_bytes(s.try_into().ok()?))
    };
    let read_u16 = |off: usize| -> Option<u16> {
        let s = elf.get(off..off + 2)?;
        Some(u16::from_le_bytes(s.try_into().ok()?))
    };

    let e_phoff = read_u64(0x20)? as usize;
    let e_phentsize = read_u16(0x36)? as usize;
    let e_phnum = read_u16(0x38)? as usize;
    if e_phentsize < MIN_PHENTSIZE {
        return None;
    }

    let mut worst: Option<u64> = None;
    let mut saw_load = false;

    for i in 0..e_phnum {
        let base = e_phoff.checked_add(i.checked_mul(e_phentsize)?)?;
        if base + MIN_PHENTSIZE > elf.len() {
            return None;
        }
        let p_type = u32::from_le_bytes(elf[base..base + 4].try_into().ok()?);
        if p_type != PT_LOAD {
            continue;
        }
        let p_memsz = u64::from_le_bytes(elf[base + 40..base + 48].try_into().ok()?);
        if p_memsz == 0 {
            continue;
        }
        let p_align = u64::from_le_bytes(elf[base + 48..base + 56].try_into().ok()?);
        saw_load = true;
        worst = Some(match worst {
            Some(current) => current.min(p_align),
            None => p_align,
        });
    }

    if !saw_load {
        return None;
    }
    let worst = worst.unwrap_or(0);
    if worst < page_size as u64 {
        Some(ElfGate::Fail {
            required_bytes: page_size,
            worst_align: worst,
        })
    } else {
        Some(ElfGate::Pass)
    }
}
