//! The Windows program carries the app's icon (build.rs): each image of
//! `assets/brand/icon.ico` is inside `gigasculpt.exe`, where Explorer and the taskbar read it.
//! Only Windows programs have one.

#[cfg(windows)]
#[test]
fn the_windows_program_carries_each_image_of_the_icon() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ico = std::fs::read(root.join("assets/brand/icon.ico")).unwrap_or_else(|e| panic!("{e}"));
    let exe = std::fs::read(env!("CARGO_BIN_EXE_gigasculpt")).unwrap_or_else(|e| panic!("{e}"));
    let word = |at: usize| u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]);
    let word = |at: usize| usize::try_from(word(at)).unwrap_or(usize::MAX);
    let count = usize::from(u16::from_le_bytes([ico[4], ico[5]]));
    assert!(count > 0, "the icon has images");
    for entry in (0..count).map(|i| 6 + 16 * i) {
        let (size, offset) = (word(entry + 8), word(entry + 12));
        let image = &ico[offset..offset + size];
        assert!(
            exe.windows(size).any(|w| w == image),
            "the program lacks the icon's image of {size} bytes at {offset}"
        );
    }
}
