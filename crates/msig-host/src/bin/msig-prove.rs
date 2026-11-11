//! One complete private `Execute` (M-of-N, N = M+2, native transfer out of `V`) through
//! `lee::execute_and_prove`, then settlement (which verifies the proof unless RISC0_DEV_MODE).
//! With the real fee path (a funded private fee payer, fee inbox) and the Undisclosed program claim,
//! so settlement classifies it as a fee-paying private tx. In-process smoke only: the benchmark
//! submits to a sequencer (`msig-bench`, scripts/bench-gpu.sh). Usage: `msig-prove [M]` (default 2).

use key_protocol::key_management::group_key_holder::GroupKeyHolder;
use lee::{AccountId, Actor};
use msig_core::seed;
use msig_host::*;

fn main() -> anyhow::Result<()> {
    let m: u8 = std::env::args().nth(1).map_or(Ok(2), |a| a.parse())?;
    let n = m + 2;
    let to = AccountId::new([0x77; 32]);
    let salt = [5; 32];
    let cfg = dev_cfg(n, m);
    let s = seed(&cfg, &salt);
    let (v, keys) = private_vault(&GroupKeyHolder::new(), s);
    let ms = initial_state(&cfg, &salt);
    let mut chain = Chain::default();
    let pre = chain.seed_vault(v, &keys, &ms, 1_000);
    chain.open_public(to, 0);
    let fp = chain.seed_fee_payer(1_000_000_000);
    let approvers: Vec<u8> = (1..=m).collect();
    anyhow::ensure!(msig_image_hex() == IMAGE_ID_V2_1, "msig image is not v2.1: build with RISC0_USE_DOCKER=1");
    let ix = signed_execute(v, &ms, native_transfer(v, to, 300), u64::MAX, &approvers);
    let w = chain.witness(v, &keys, s, Some(pre));
    let fee_witness = chain.fee_witness(&fp);
    eprintln!(
        "proving private Execute M={m} N={n} (RISC0_DEV_MODE={:?})",
        std::env::var("RISC0_DEV_MODE").ok()
    );
    let (tx, took) = chain
        .prove_tx(
            vec![w, fee_witness],
            Actor::new(v, program_account()),
            borsh::to_vec(&ix)?,
            vec![Actor::native_balance(to), Actor::native_balance(fp.inbox)],
            Some(fp.transfer(100_000_000)),
            &chain.msig_catalog(),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    chain
        .settle(&tx)
        .map_err(|e| anyhow::anyhow!("settle: {e}"))?;
    anyhow::ensure!(chain.public_balance(to) == 300, "recipient not credited");
    println!(
        "RESULT m={m} n={n} sigs={} execute_and_prove_s={:.2} tx_bytes={} settled=true image_id={} dev_mode={}",
        m + 2,
        took.as_secs_f64(),
        borsh::to_vec(&tx)?.len(),
        msig_image_hex(),
        std::env::var("RISC0_DEV_MODE").is_ok(),
    );
    Ok(())
}
