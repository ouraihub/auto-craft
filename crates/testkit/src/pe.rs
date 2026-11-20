//! Small PE32+ builder following craft-anything's testkit `pe64` positioned-header layout.
//! This analysis-only sample retains `puts`/`helper` import/export evidence, without resources.

/// Builds a deterministic AMD64 console PE importing `msvcrt.dll!puts` and exporting `helper`.
/// The entry point prints `Hello, Craft!`; `helper` returns 42. No compiler or binary asset needed.
/// Reference: storyTold/craft-anything `crates/testkit/src/lib.rs`, `pe64` (MIT OR Apache-2.0).
/// Tests analyze this fixture; they must not execute it.
pub fn pe64() -> Vec<u8> {
    let mut bytes = vec![0; 0x600];
    put(&mut bytes, 0, b"MZ");
    u32_at(&mut bytes, 0x3c, 0x80);
    put(&mut bytes, 0x80, b"PE\0\0");
    let coff = 0x84;
    u16_at(&mut bytes, coff, 0x8664);
    u16_at(&mut bytes, coff + 2, 2);
    u32_at(&mut bytes, coff + 4, 0x6000_0000);
    u16_at(&mut bytes, coff + 16, 240);
    u16_at(&mut bytes, coff + 18, 0x23); // executable, large-address-aware, fixed base
    let opt = coff + 20;
    u16_at(&mut bytes, opt, 0x20b);
    u32_at(&mut bytes, opt + 4, 0x200);
    u32_at(&mut bytes, opt + 8, 0x200);
    u32_at(&mut bytes, opt + 16, 0x1000);
    u32_at(&mut bytes, opt + 20, 0x1000);
    u64_at(&mut bytes, opt + 24, 0x1_4000_0000);
    u32_at(&mut bytes, opt + 32, 0x1000);
    u32_at(&mut bytes, opt + 36, 0x200);
    u16_at(&mut bytes, opt + 40, 6);
    u16_at(&mut bytes, opt + 48, 6);
    u32_at(&mut bytes, opt + 56, 0x3000);
    u32_at(&mut bytes, opt + 60, 0x200);
    u16_at(&mut bytes, opt + 68, 3);
    u16_at(&mut bytes, opt + 70, 0x8100); // NX, terminal-server-aware; no ASLR without relocations
    u64_at(&mut bytes, opt + 72, 0x100000);
    u64_at(&mut bytes, opt + 80, 0x1000);
    u64_at(&mut bytes, opt + 88, 0x100000);
    u64_at(&mut bytes, opt + 96, 0x1000);
    u32_at(&mut bytes, opt + 108, 16);
    let directories = opt + 112;
    for (index, rva, size) in [(0, 0x21a0, 0x60), (1, 0x2100, 40), (12, 0x2160, 16)] {
        u32_at(&mut bytes, directories + index * 8, rva);
        u32_at(&mut bytes, directories + index * 8 + 4, size);
    }
    let sections = opt + 240;
    for (index, name, rva, raw, flags) in [
        (0, b".text".as_slice(), 0x1000, 0x200, 0x6000_0020),
        (1, b".rdata".as_slice(), 0x2000, 0x400, 0x4000_0040),
    ] {
        let header = sections + index * 40;
        put(&mut bytes, header, name);
        u32_at(&mut bytes, header + 8, 0x200);
        u32_at(&mut bytes, header + 12, rva);
        u32_at(&mut bytes, header + 16, 0x200);
        u32_at(&mut bytes, header + 20, raw);
        u32_at(&mut bytes, header + 36, flags);
    }
    // Windows x64 ABI: shadow space and stack alignment, rcx=string, indirect call through IAT.
    put(
        &mut bytes,
        0x200,
        &[
            0x48, 0x83, 0xec, 0x28, 0x48, 0x8d, 0x0d, 0xf5, 0x0f, 0x00, 0x00, 0xff, 0x15, 0x4f,
            0x11, 0x00, 0x00, 0x31, 0xc0, 0x48, 0x83, 0xc4, 0x28, 0xc3,
        ],
    );
    put(&mut bytes, 0x240, &[0xb8, 42, 0, 0, 0, 0xc3]);
    put(&mut bytes, 0x400, b"Hello, Craft!\0");
    put(&mut bytes, 0x410, b"open_settings_panel\0");
    u32_at(&mut bytes, 0x500, 0x2140); // import lookup table
    u32_at(&mut bytes, 0x50c, 0x2190); // DLL name
    u32_at(&mut bytes, 0x510, 0x2160); // import address table
    u64_at(&mut bytes, 0x540, 0x2180);
    u64_at(&mut bytes, 0x560, 0x2180);
    put(&mut bytes, 0x582, b"puts\0");
    put(&mut bytes, 0x590, b"msvcrt.dll\0");
    for (offset, value) in [
        (12, 0x21e0),
        (16, 1),
        (20, 1),
        (24, 1),
        (28, 0x21d0),
        (32, 0x21d4),
        (36, 0x21d8),
    ] {
        u32_at(&mut bytes, 0x5a0 + offset, value);
    }
    u32_at(&mut bytes, 0x5d0, 0x1040);
    u32_at(&mut bytes, 0x5d4, 0x21f0);
    put(&mut bytes, 0x5e0, b"sample.exe\0");
    put(&mut bytes, 0x5f0, b"helper\0");
    bytes
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

fn u16_at(bytes: &mut [u8], offset: usize, value: u16) {
    put(bytes, offset, &value.to_le_bytes());
}

fn u32_at(bytes: &mut [u8], offset: usize, value: u32) {
    put(bytes, offset, &value.to_le_bytes());
}

fn u64_at(bytes: &mut [u8], offset: usize, value: u64) {
    put(bytes, offset, &value.to_le_bytes());
}
