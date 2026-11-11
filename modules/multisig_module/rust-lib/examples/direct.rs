//! Fallback / smoke: the module's `core` functions without the Logos host.
//! usage: direct create <home> <members> <threshold> <fund> | direct execute <home> <label> <amount> <signers>
use multisig_module::core;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let r = match a.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["create", h, n, m, f] => core::create(h, n.parse().unwrap(), m.parse().unwrap(), f),
        ["execute", h, l, amt, s] => core::execute(h, l, amt, s),
        _ => panic!("usage: direct create <home> <members> <threshold> <fund> | direct execute <home> <label> <amount> <signers>"),
    };
    println!("{r}");
    std::process::exit(if r["ok"] == true { 0 } else { 1 });
}
