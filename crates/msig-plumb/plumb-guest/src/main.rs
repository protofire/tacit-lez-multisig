//! BENCH-ONLY, never deployed. The `Execute { action: Call }` path of `msig_core::handle` with the
//! M+2 signature verifications removed: everything else is the same work (input
//! frame, borsh of state and instruction, the three digests, state re-encode, response). Cycles of
//! `msig` minus cycles of this = the crypto cost. shortcut: Call actions only, which is all the
//! benchmark sends.
use lee_core::program::{PdaSeed, Response, SyncMessage, run_actor_with};
use msig_core::{Action, Instruction, Multisig, execute_digests};

fn main() {
    run_actor_with(|input| {
        let Instruction::Execute { action, expiry, approvals, .. } =
            borsh::from_slice(&input.message).expect("instruction")
        else {
            panic!("Execute only")
        };
        let mut ms: Multisig = borsh::from_slice(&input.pre_state).expect("state");
        // the digests are computed (and kept live) exactly as check_quorum does, but never verified
        let d = execute_digests(&input.receiver.account_id, &ms, &action, expiry);
        assert!(approvals.len() >= usize::from(ms.threshold) && d[0] != d[1]);
        ms.nonce += 1;
        let Action::Call { to, data, with_authority } = action else { panic!("Call only") };
        let call = SyncMessage { to, message: data, pda_seeds: Default::default() };
        let call = if with_authority { call.with_pda_seeds([PdaSeed::new(ms.seed)]) } else { call };
        Response::set_state(borsh::to_vec(&ms).expect("borsh")).send(call).timestamp_window(..expiry)
    })
}
