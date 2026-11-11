//! Same exports as crates/msig-methods, from the committed guest build (msig.bin, sha256 f10a7a77..., image v2.1: Docker build of 749e9d3's guest sources, see evidence/IMAGE-ID-v2.1.md).
//! The module build cannot run the risc0 guest toolchain; `image_id_matches_elf` re-derives the id.
pub const MSIG_ELF: &[u8] = include_bytes!("../msig.bin");
pub const MSIG_ID: [u32; 8] = [1604613345, 3131579592, 129300626, 4215690626, 3245835187, 919914509, 1847985186, 1464144892];

// msig-host re-exports the test-only noop guest; the module never uses it.
pub const NOOP_ELF: &[u8] = &[];
pub const NOOP_ID: [u32; 8] = [0; 8];

#[cfg(test)]
mod tests {
    #[test]
    fn image_id_matches_elf() {
        let id = risc0_binfmt::ProgramBinary::decode(super::MSIG_ELF).unwrap().compute_image_id().unwrap();
        assert_eq!(id.as_words(), super::MSIG_ID);
    }
}
