//! P.2: executor cycles and segments of the `msig` guest for an `Execute` at M approvals
//! (M+2 signatures checked), and of the signature-free "plumbing" guest on the same input.
//! Executor only (no proving). Usage: `msig-cycles [M ...]` (default 1 2 5 10 16 24 28).
//! N = min(M+2, MAX_MEMBERS); approvers are members 0..M, proposer and executor member 0.

use lee::{AccountId, Actor};
use lee_core::program::ReceiveInput;
use msig_host::*;
use risc0_zkvm::{ExecutorEnv, default_executor};

fn run(name: &str, elf: &[u8], input: &ReceiveInput, m: u8, n: u8) -> anyhow::Result<()> {
    let mut env = ExecutorEnv::builder();
    env.write_slice(&lee_core::to_frame(&borsh::to_vec(input)?));
    let info = default_executor().execute(env.build()?, elf)?;
    anyhow::ensure!(
        matches!(info.exit_code, risc0_zkvm::ExitCode::Halted(0)),
        "{name} M={m}: exit {:?}",
        info.exit_code
    );
    let padded: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
    println!(
        "CYCLES guest={name} m={m} n={n} sigs={} user_cycles={} segments={} padded_cycles={padded}",
        m + 2,
        info.cycles(),
        info.segments.len(),
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let ms_list: Vec<u8> = {
        let a: Vec<u8> = std::env::args().skip(1).map(|a| a.parse()).collect::<Result<_, _>>()?;
        if a.is_empty() { vec![1, 2, 5, 10, 16, 24, 28] } else { a }
    };
    anyhow::ensure!(msig_image_hex() == IMAGE_ID_V2_1, "msig image is not v2.1: build with RISC0_USE_DOCKER=1");
    let (v, to) = (AccountId::new([0x55; 32]), AccountId::new([0x77; 32]));
    for m in ms_list {
        let n = (m + 2).min(msig_core::MAX_MEMBERS as u8);
        let cfg = dev_cfg(n, m);
        let ms = initial_state(&cfg, &[5; 32]);
        let approvers: Vec<u8> = (0..m).collect();
        let ix = signed_execute(v, &ms, native_transfer(v, to, 1), u64::MAX, &approvers);
        let input = ReceiveInput {
            receiver: Actor::new(v, program_account()),
            from: None,
            is_authorized: true,
            pre_state: borsh::to_vec(&ms)?.into(),
            message: borsh::to_vec(&ix)?.into(),
        };
        run("msig", MSIG_ELF, &input, m, n)?;
        run("plumb", msig_plumb::PLUMB_GUEST_ELF, &input, m, n)?;
    }
    println!(
        "IMAGES msig={} plumb={} (plumb is bench-only, never deployed)",
        hex::encode(risc0_zkvm::sha::Digest::from(MSIG_ID)),
        hex::encode(risc0_zkvm::sha::Digest::from(msig_plumb::PLUMB_GUEST_ID)),
    );
    Ok(())
}
