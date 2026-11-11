//! Rust bindings for rapidsnark proving.
//!
//! Prebuilt binaries are provided for the following platforms:
//! - aarch64-apple-ios
//! - aarch64-apple-ios-sim
//! - x86_64-apple-ios
//! - aarch64-apple-darwin
//! - x86_64-apple-darwin
//! - aarch64-linux-android
//! - x86_64-linux-android
//! - x86_64 linux
//! - arm64 linux
//!
//! If a specific target is not included the sysytem will fallback to
//! the generic architecture, which may cause problems. e.g. if you compile
//! for aarch64-linux-generic, the system will fallback to aarch64.
//!

use std::collections::HashMap;
use std::ffi::{c_char, c_ulonglong, c_void};
use std::str::FromStr;

use anyhow::Result;
use num_bigint::BigInt;

/// A function that converts named inputs to a full witness. This should be generated using e.g.
/// [rust-witness](https://crates.io/crates/rust-witness).
pub type WtnsFn = fn(HashMap<String, Vec<BigInt>>) -> Vec<BigInt>;

/// A structure representing a proof and public signals.
#[derive(Debug)]
pub struct ProofResult {
    pub proof: String,
    pub public_signals: String,
}

const PROVER_OK: i32 = 0;
const PROVER_ERROR_SHORT_BUFFER: i32 = 2;

extern "C" {
    pub fn groth16_public_size_for_zkey_buf(
        zkey_buffer: *const c_void,
        zkey_size: c_ulonglong,
        public_size: *mut c_ulonglong,
        error_msg: *mut c_char,
        error_msg_maxsize: c_ulonglong,
    ) -> i32;

    pub fn groth16_proof_size(proof_size: *mut c_ulonglong);

    pub fn groth16_prover(
        zkey_buffer: *const c_void,
        zkey_size: c_ulonglong,
        wtns_buffer: *const c_void,
        wtns_size: c_ulonglong,
        proof_buffer: *mut c_char,
        proof_size: *mut c_ulonglong,
        public_buffer: *mut c_char,
        public_size: *mut c_ulonglong,
        error_msg: *mut c_char,
        error_msg_maxsize: c_ulonglong,
    ) -> i32;

    pub fn groth16_prover_zkey_file(
        zkey_file_path: *const std::os::raw::c_char,
        wtns_buffer: *const std::os::raw::c_void,
        wtns_size: c_ulonglong,
        proof_buffer: *mut std::os::raw::c_char,
        proof_size: *mut c_ulonglong,
        public_buffer: *mut std::os::raw::c_char,
        public_size: *mut c_ulonglong,
        error_msg: *mut std::os::raw::c_char,
        error_msg_maxsize: c_ulonglong,
    ) -> i32;

    pub fn groth16_verify(
        proof: *const std::os::raw::c_char,
        inputs: *const std::os::raw::c_char,
        verification_key: *const std::os::raw::c_char,
        error_msg: *mut std::os::raw::c_char,
        error_msg_maxsize: std::ffi::c_ulong,
    ) -> i32;
}

use num_traits::ops::bytes::ToBytes;
use std::io::{self};

/// Parse bigints to `wtns` format.<br/>
/// Reference: [witnesscalc/src/witnesscalc.cpp](https://github.com/0xPolygonID/witnesscalc/blob/4a789880727aa0df50f1c4ef78ec295f5a30a15e/src/witnesscalc.cpp)
pub fn parse_bigints_to_witness(bigints: Vec<BigInt>) -> io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    let version: u32 = 2;
    let n_sections: u32 = 2;
    let n8: u32 = 32;
    let q = BigInt::from_str(
        "21888242871839275222246405745257275088548364400416034343698204186575808495617",
    )
    .unwrap();
    let n_witness_values: u32 = bigints.len() as u32;

    // Write the format bytes (4 bytes)
    let wtns_format = "wtns".as_bytes();
    buffer.extend_from_slice(wtns_format);

    // Write version (4 bytes)
    buffer.extend_from_slice(&version.to_le_bytes());

    // Write number of sections (4 bytes)
    buffer.extend_from_slice(&n_sections.to_le_bytes());

    // Iterate through sections to write the data
    // Section 1 (Field parameters)
    let section_id_1: u32 = 1;
    let section_length_1: u64 = 8 + n8 as u64;
    buffer.extend_from_slice(&section_id_1.to_le_bytes());
    buffer.extend_from_slice(&section_length_1.to_le_bytes());

    // Write n8 (4 bytes), q (32 bytes), and n_witness_values (4 bytes)
    buffer.extend_from_slice(&n8.to_le_bytes());
    buffer.extend_from_slice(&q.to_signed_bytes_le());
    buffer.extend_from_slice(&n_witness_values.to_le_bytes());

    // Section 2 (Witness data)
    let section_id_2: u32 = 2;
    let section_length_2: u64 = bigints.len() as u64 * n8 as u64; // Witness data size
    buffer.extend_from_slice(&section_id_2.to_le_bytes());
    buffer.extend_from_slice(&section_length_2.to_le_bytes());

    // Write the witness data (each BigInt to n8 bytes)
    for bigint in bigints {
        let mut bytes = bigint.to_le_bytes();
        bytes.resize(n8 as usize, 0); // Ensure each BigInt is padded to n8 bytes
        buffer.extend_from_slice(&bytes);
    }

    // Return the buffer containing the complete witness data
    Ok(buffer)
}

fn error_message(error_msg: &[u8]) -> String {
    let end = error_msg
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(error_msg.len());
    String::from_utf8_lossy(&error_msg[..end]).into_owned()
}

fn buffer_len(size: c_ulonglong, name: &str) -> Result<usize> {
    usize::try_from(size).map_err(|_| anyhow::anyhow!("{name} buffer size is too large"))
}

fn buffer_len_with_null(size: c_ulonglong, name: &str) -> Result<usize> {
    let size = size
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("{name} buffer size overflowed"))?;
    buffer_len(size, name)
}

fn output_string(name: &str, buffer: &[u8], size: c_ulonglong) -> Result<String> {
    let size = buffer_len(size, name)?;
    if size > buffer.len() {
        return Err(anyhow::anyhow!(
            "{name} output size {size} exceeds buffer size {}",
            buffer.len()
        ));
    }
    String::from_utf8(buffer[..size].to_vec())
        .map_err(|err| anyhow::anyhow!("{name} output is not valid UTF-8: {err}"))
}

/// Wrapper for `groth16_prover_zkey_file`
pub fn groth16_prover_zkey_file_wrapper(
    zkey_path: &str,
    wtns_buffer: Vec<u8>,
) -> Result<ProofResult> {
    let formatted_zkey_path = std::ffi::CString::new(zkey_path).unwrap();
    let wtns_size = wtns_buffer.len() as u64;

    let mut proof_buffer = vec![0u8; 4 * 1024 * 1024]; // Adjust size as needed
    let mut proof_size: u64 = 4 * 1024 * 1024;
    let proof_ptr = proof_buffer.as_mut_ptr() as *mut std::ffi::c_char;

    let mut public_buffer = vec![0u8; 4 * 1024 * 1024]; // Adjust size as needed
    let mut public_size: u64 = 4 * 1024 * 1024;
    let public_ptr = public_buffer.as_mut_ptr() as *mut std::ffi::c_char;

    let mut error_msg = vec![0u8; 256]; // Error message buffer
    let error_msg_ptr = error_msg.as_mut_ptr() as *mut std::ffi::c_char;

    unsafe {
        let result = groth16_prover_zkey_file(
            formatted_zkey_path.as_ptr() as *const std::ffi::c_char,
            wtns_buffer.as_ptr() as *const std::os::raw::c_void, // Witness buffer
            wtns_size,
            proof_ptr,
            &mut proof_size,
            public_ptr,
            &mut public_size,
            error_msg_ptr,
            error_msg.len() as u64,
        );
        if result != 0 {
            let error_string = std::ffi::CStr::from_ptr(error_msg_ptr)
                .to_string_lossy()
                .into_owned();
            return Err(anyhow::anyhow!("Proof generation failed: {}", error_string));
        }
        // Convert both strings
        let proof = std::ffi::CStr::from_ptr(proof_ptr)
            .to_string_lossy()
            .into_owned();
        let public_signals = std::ffi::CStr::from_ptr(public_ptr)
            .to_string_lossy()
            .into_owned();
        Ok(ProofResult {
            proof,
            public_signals,
        })
    }
}

/// Wrapper for `groth16_prover`, which proves from an in-memory zkey buffer.
pub fn groth16_prover_zkey_buffer_wrapper(
    zkey_buffer: &[u8],
    wtns_buffer: &[u8],
) -> Result<ProofResult> {
    let zkey_size = zkey_buffer.len() as c_ulonglong;
    let wtns_size = wtns_buffer.len() as c_ulonglong;

    let mut error_msg = vec![0u8; 256];
    let error_msg_ptr = error_msg.as_mut_ptr() as *mut c_char;

    let mut proof_size: c_ulonglong = 0;
    let mut public_size: c_ulonglong = 0;

    unsafe {
        groth16_proof_size(&mut proof_size);

        let result = groth16_public_size_for_zkey_buf(
            zkey_buffer.as_ptr() as *const c_void,
            zkey_size,
            &mut public_size,
            error_msg_ptr,
            error_msg.len() as c_ulonglong,
        );
        if result != PROVER_OK {
            return Err(anyhow::anyhow!(
                "Public signal buffer size calculation failed: {}",
                error_message(&error_msg)
            ));
        }
    }

    let mut proof_buffer = vec![0u8; buffer_len_with_null(proof_size, "proof")?];
    let mut public_buffer = vec![0u8; buffer_len_with_null(public_size, "public signals")?];

    let mut proof_output_size = proof_buffer.len() as c_ulonglong;
    let mut public_output_size = public_buffer.len() as c_ulonglong;

    let prove_once = |proof_buffer: &mut [u8],
                      proof_output_size: &mut c_ulonglong,
                      public_buffer: &mut [u8],
                      public_output_size: &mut c_ulonglong,
                      error_msg: &mut [u8]| {
        error_msg.fill(0);
        unsafe {
            groth16_prover(
                zkey_buffer.as_ptr() as *const c_void,
                zkey_size,
                wtns_buffer.as_ptr() as *const c_void,
                wtns_size,
                proof_buffer.as_mut_ptr() as *mut c_char,
                proof_output_size,
                public_buffer.as_mut_ptr() as *mut c_char,
                public_output_size,
                error_msg.as_mut_ptr() as *mut c_char,
                error_msg.len() as c_ulonglong,
            )
        }
    };

    let mut result = prove_once(
        &mut proof_buffer,
        &mut proof_output_size,
        &mut public_buffer,
        &mut public_output_size,
        &mut error_msg,
    );

    if result == PROVER_ERROR_SHORT_BUFFER {
        proof_buffer.resize(buffer_len_with_null(proof_output_size, "proof")?, 0);
        public_buffer.resize(
            buffer_len_with_null(public_output_size, "public signals")?,
            0,
        );
        proof_output_size = proof_buffer.len() as c_ulonglong;
        public_output_size = public_buffer.len() as c_ulonglong;

        result = prove_once(
            &mut proof_buffer,
            &mut proof_output_size,
            &mut public_buffer,
            &mut public_output_size,
            &mut error_msg,
        );
    }

    if result != PROVER_OK {
        return Err(anyhow::anyhow!(
            "Proof generation failed: {}",
            error_message(&error_msg)
        ));
    }

    Ok(ProofResult {
        proof: output_string("proof", &proof_buffer, proof_output_size)?,
        public_signals: output_string("public signals", &public_buffer, public_output_size)?,
    })
}

/// Wrapper for `groth16_verify`
pub fn groth16_verify_wrapper(proof: &str, inputs: &str, verification_key: &str) -> Result<bool> {
    let proof_cstr = std::ffi::CString::new(proof).unwrap();
    let inputs_cstr = std::ffi::CString::new(inputs).unwrap();
    let verification_key_cstr = std::ffi::CString::new(verification_key).unwrap();

    let mut error_msg = vec![0u8; 256]; // Error message buffer
    let error_msg_ptr = error_msg.as_mut_ptr() as *mut std::ffi::c_char;
    unsafe {
        let result = groth16_verify(
            proof_cstr.as_ptr() as *const std::ffi::c_char,
            inputs_cstr.as_ptr() as *const std::ffi::c_char,
            verification_key_cstr.as_ptr() as *const std::ffi::c_char,
            error_msg_ptr,
            error_msg.len() as u64,
        );
        if result == 2 {
            let error_string = std::ffi::CStr::from_ptr(error_msg_ptr)
                .to_string_lossy()
                .into_owned();
            return Err(anyhow::anyhow!(
                "Proof verification failed: {}",
                error_string
            ));
        }
        Ok(result == 0)
    }
}
