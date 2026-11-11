//! Prints the multisig guest ImageID (hex of the digest bytes). Used by `scripts/image-ids.sh`.
fn main() {
    let hex: String = msig_methods::MSIG_ID
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("msig {hex}");
}
