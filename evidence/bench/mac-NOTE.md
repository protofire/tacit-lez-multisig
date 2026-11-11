# Mac benchmark note (image v2.1)

- Machine: Apple M2 Max, 32 GB, macOS 27.2. Not rebooted for 11 days; swap ~31 GB used; other apps open (`mac-metal.log`; superseded by the clean re-run below).
- Both `mac-cpu.log` and `mac-metal.log` ran the CPU prover. In risc0-circuit-rv32im 4.0.5 (pulled by risc0-zkvm 3.0.5, pinned by LEZ v0.4.0-rc1) the Apple GPU branch of `segment_prover()` is commented out (`src/prove/mod.rs:49-50`), and risc0-zkvm marks the `metal` feature deprecated. The matching timings of the two variants confirm it. There is no Metal path to measure on this stack.
- Build workaround (host only, guest image unchanged): the macOS 27 Metal compiler rejects `cells[i] = 0` on `Fp` in risc0-sys 1.5.0 `poseidon2.metal`; `scripts/bench-mac.sh` builds against a copy with `= Fp(0)`.

## Clean re-run (`mac-cpu.log`)

- After a reboot: swap 0, Docker stopped, 79% memory free at start. CPU variant only (Metal is the same path, see above).
- Core diagnostic: system-wide CPU 1062–1094% average per proving step (≈10.7 of 12 cores; 8P+4E), so the prover uses the whole machine.
- A video call ran on the machine during Execute M=5 runs 2–3 and Create M=10 (visible as slower runs at the same total CPU). Reported figures: Execute M=5 = run 1 (738.6 s proving, 768.7 s e2e); Execute M=10 = median of runs 1–3 (899.9 s proving, 930.1 s e2e; range 896.9–926.7). Create M=5 588.1 s, fund M=5 535.9 s. Peak RSS ≤ 12.3 GB.
- The earlier loaded session (before reboot, swap full) is kept as `mac-metal.log`: Execute M=5 ≈ 766 s, M=10 ≈ 917 s.
