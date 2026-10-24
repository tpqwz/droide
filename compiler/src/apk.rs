//! APK container and APK Signature Scheme v2 (min SDK 24, v2 only).

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use flate2::write::DeflateEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::error::CompileError;

const SIG_BLOCK_ID: u32 = 0x7109871A;
const SIG_MAGIC: &[u8] = b"APK Sig Block 42";
const ALGO_RSA_PKCS1_SHA256: u32 = 0x0103;
const CHUNK: usize = 1024 * 1024;
const ZIP64_LIMIT: u64 = (1 << 31) - 1;
const ZIP_FILECOUNT_LIMIT: usize = (1 << 16) - 1;
const VERSION: u16 = 20;
const METHOD_DEFLATE: u16 = 8;
/// 1980-01-01 00:00:00 in DOS format.
const DOS_DATE: u16 = (1 << 5) | 1;
const DOS_TIME: u16 = 0;
const EXTERNAL_ATTR: u32 = 0o644 << 16;

const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            if crc & 1 != 0 {
                crc = 0xEDB88320 ^ (crc >> 1);
            } else {
                crc >>= 1;
            }
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let index = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = CRC_TABLE[index] ^ (crc >> 8);
    }
    !crc
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn run_openssl(mut cmd: Command, input: Option<&[u8]>) -> Result<Vec<u8>, CompileError> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|err| CompileError::bare(err.to_string()))?;
    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data);
        }
    } else {
        drop(child.stdin.take());
    }
    let output = child
        .wait_with_output()
        .map_err(|err| CompileError::bare(err.to_string()))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim();
        if message.is_empty() {
            return Err(CompileError::bare("openssl failed"));
        }
        return Err(CompileError::bare(message.to_string()));
    }
    Ok(output.stdout)
}

fn debug_key() -> Result<(PathBuf, Vec<u8>, Vec<u8>), CompileError> {
    let home = env::var_os("HOME").ok_or_else(|| CompileError::bare("HOME is not set"))?;
    let dir = PathBuf::from(home).join(".local/share/droide");
    fs::create_dir_all(&dir).map_err(|err| CompileError::bare(err.to_string()))?;
    let key = dir.join("debug.key.pem");
    let cert = dir.join("debug.crt.pem");
    if !key.exists() || !cert.exists() {
        let mut cmd = Command::new("openssl");
        cmd.args(["req", "-newkey", "rsa:2048", "-keyout"])
            .arg(&key)
            .args(["-x509", "-days", "3650", "-nodes", "-out"])
            .arg(&cert)
            .args(["-subj", "/CN=Droide Debug"]);
        run_openssl(cmd, None)?;
    }
    let mut public_cmd = Command::new("openssl");
    public_cmd
        .args(["pkey", "-in"])
        .arg(&key)
        .args(["-pubout", "-outform", "DER"]);
    let public = run_openssl(public_cmd, None)?;
    let mut cert_cmd = Command::new("openssl");
    cert_cmd
        .args(["x509", "-in"])
        .arg(&cert)
        .args(["-outform", "DER"]);
    let certificate = run_openssl(cert_cmd, None)?;
    Ok((key, public, certificate))
}

fn lp(blob: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + blob.len());
    push_u32(&mut out, blob.len() as u32);
    out.extend_from_slice(blob);
    out
}

/// Each part length-prefixed, no outer length.
fn seq(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend(lp(part));
    }
    out
}

/// One length-prefixed (algorithm id, length-prefixed value) record.
fn pairs(algo: u32, value: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(4 + 4 + value.len());
    push_u32(&mut inner, algo);
    inner.extend(lp(value));
    lp(&inner)
}

fn content_digest(sections: &[&[u8]]) -> [u8; 32] {
    let mut digests: Vec<[u8; 32]> = Vec::new();
    for section in sections {
        let mut offset = 0;
        while offset < section.len() {
            let end = (offset + CHUNK).min(section.len());
            let chunk = &section[offset..end];
            offset = end;
            let mut hasher = Sha256::new();
            hasher.update([0xa5]);
            hasher.update((chunk.len() as u32).to_le_bytes());
            hasher.update(chunk);
            let digest = hasher.finalize();
            let mut item = [0u8; 32];
            item.copy_from_slice(&digest);
            digests.push(item);
        }
    }
    let mut top = Sha256::new();
    top.update([0x5a]);
    top.update((digests.len() as u32).to_le_bytes());
    for item in &digests {
        top.update(item);
    }
    let digest = top.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn find_eocd(blob: &[u8]) -> Result<usize, CompileError> {
    let start = blob.len().saturating_sub(22 + 65535);
    blob[start..]
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .map(|index| start + index)
        .ok_or_else(|| CompileError::bare("ZIP end of central directory not found"))
}

fn signing_block(v2_value: &[u8]) -> Vec<u8> {
    let mut pair = Vec::with_capacity(12 + v2_value.len());
    push_u64(&mut pair, 4 + v2_value.len() as u64);
    push_u32(&mut pair, SIG_BLOCK_ID);
    pair.extend_from_slice(v2_value);
    let size = (pair.len() + 8 + 16) as u64;
    let mut out = Vec::with_capacity(8 + pair.len() + 8 + SIG_MAGIC.len());
    push_u64(&mut out, size);
    out.extend_from_slice(&pair);
    push_u64(&mut out, size);
    out.extend_from_slice(SIG_MAGIC);
    out
}

fn sign_with(key: &Path, signed_data: &[u8]) -> Result<Vec<u8>, CompileError> {
    let mut cmd = Command::new("openssl");
    cmd.args(["dgst", "-sha256", "-sign"]).arg(key);
    run_openssl(cmd, Some(signed_data))
}

fn sign_apk(unsigned: &[u8]) -> Result<Vec<u8>, CompileError> {
    let eocd_off = find_eocd(unsigned)?;
    if eocd_off + 22 > unsigned.len() {
        return Err(CompileError::bare("ZIP end of central directory not found"));
    }
    let eocd = &unsigned[eocd_off..];
    let cd_size = u32::from_le_bytes(eocd[12..16].try_into().unwrap()) as usize;
    let cd_off = u32::from_le_bytes(eocd[16..20].try_into().unwrap()) as usize;
    if cd_off
        .checked_add(cd_size)
        .map_or(true, |end| end > eocd_off)
    {
        return Err(CompileError::bare("ZIP central directory is out of range"));
    }
    let before = &unsigned[..cd_off];
    let central = &unsigned[cd_off..cd_off + cd_size];
    let (key, public, certificate) = debug_key()?;

    // Digest as if the signing block were absent. The real EOCD offset includes it.
    let mut patched = eocd.to_vec();
    let digest_off = u32::try_from(before.len())
        .map_err(|_| CompileError::bare("Zipfile size would require ZIP64 extensions"))?;
    patched[16..20].copy_from_slice(&digest_off.to_le_bytes());
    let apk_digest = content_digest(&[before, central, &patched]);

    let digests = pairs(ALGO_RSA_PKCS1_SHA256, &apk_digest);
    let certs = seq(&[&certificate]);
    let signed_data = seq(&[&digests, &certs, &[], &[]]);
    let signature = sign_with(&key, &signed_data)?;
    let signatures = pairs(ALGO_RSA_PKCS1_SHA256, &signature);
    let signer = seq(&[&signed_data, &signatures, &public]);
    let signers = seq(&[&signer]);
    let v2 = seq(&[&signers]);
    let block = signing_block(&v2);

    let mut eocd_out = eocd.to_vec();
    let cd_offset = u32::try_from(before.len() + block.len())
        .map_err(|_| CompileError::bare("Zipfile size would require ZIP64 extensions"))?;
    eocd_out[16..20].copy_from_slice(&cd_offset.to_le_bytes());

    let mut out = Vec::with_capacity(before.len() + block.len() + central.len() + eocd_out.len());
    out.extend_from_slice(before);
    out.extend_from_slice(&block);
    out.extend_from_slice(central);
    out.extend_from_slice(&eocd_out);
    Ok(out)
}

fn deflate(data: &[u8]) -> Result<Vec<u8>, CompileError> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(6));
    encoder
        .write_all(data)
        .map_err(|err| CompileError::bare(err.to_string()))?;
    encoder
        .finish()
        .map_err(|err| CompileError::bare(err.to_string()))
}

fn encode_filename(name: &str) -> (Vec<u8>, u16) {
    let name = name.split('\0').next().unwrap_or("");
    if name.is_ascii() {
        (name.as_bytes().to_vec(), 0)
    } else {
        (name.as_bytes().to_vec(), 0x800)
    }
}

struct ZipEntry {
    name: Vec<u8>,
    flags: u16,
    crc: u32,
    size: u32,
    compressed_size: u32,
    offset: u32,
}

fn zip_limit(value: u64, message: &str) -> Result<(), CompileError> {
    if value > ZIP64_LIMIT {
        Err(CompileError::bare(message))
    } else {
        Ok(())
    }
}

fn write_local(out: &mut Vec<u8>, entry: &ZipEntry) {
    out.extend_from_slice(b"PK\x03\x04");
    push_u16(out, VERSION);
    push_u16(out, entry.flags);
    push_u16(out, METHOD_DEFLATE);
    push_u16(out, DOS_TIME);
    push_u16(out, DOS_DATE);
    push_u32(out, entry.crc);
    push_u32(out, entry.compressed_size);
    push_u32(out, entry.size);
    push_u16(out, entry.name.len() as u16);
    push_u16(out, 0);
    out.extend_from_slice(&entry.name);
}

fn write_central(out: &mut Vec<u8>, entry: &ZipEntry) {
    out.extend_from_slice(b"PK\x01\x02");
    push_u16(out, VERSION);
    push_u16(out, VERSION);
    push_u16(out, entry.flags);
    push_u16(out, METHOD_DEFLATE);
    push_u16(out, DOS_TIME);
    push_u16(out, DOS_DATE);
    push_u32(out, entry.crc);
    push_u32(out, entry.compressed_size);
    push_u32(out, entry.size);
    push_u16(out, entry.name.len() as u16);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u32(out, EXTERNAL_ATTR);
    push_u32(out, entry.offset);
    out.extend_from_slice(&entry.name);
}

fn write_eocd(out: &mut Vec<u8>, count: u16, cd_size: u32, cd_off: u32) {
    out.extend_from_slice(b"PK\x05\x06");
    push_u16(out, 0);
    push_u16(out, 0);
    push_u16(out, count);
    push_u16(out, count);
    push_u32(out, cd_size);
    push_u32(out, cd_off);
    push_u16(out, 0);
}

/// entries: archive order. ZIP_DEFLATED level 6, 1980-01-01, create_system 0, mode 0644, no ZIP64.
fn zip_entries(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, CompileError> {
    if entries.len() > ZIP_FILECOUNT_LIMIT {
        return Err(CompileError::bare(
            "Files count would require ZIP64 extensions",
        ));
    }
    let mut out = Vec::new();
    let mut stored = Vec::with_capacity(entries.len());
    for (name, data) in entries {
        zip_limit(data.len() as u64, "Filesize would require ZIP64 extensions")?;
        let (name_bytes, flags) = encode_filename(name);
        if name_bytes.len() > u16::MAX as usize {
            return Err(CompileError::bare(format!("file name too long: {name}")));
        }
        zip_limit(
            out.len() as u64,
            "Zipfile size would require ZIP64 extensions",
        )?;
        let compressed = deflate(data)?;
        zip_limit(
            compressed.len() as u64,
            "Filesize would require ZIP64 extensions",
        )?;
        let entry = ZipEntry {
            name: name_bytes,
            flags,
            crc: crc32(data),
            size: data.len() as u32,
            compressed_size: compressed.len() as u32,
            offset: out.len() as u32,
        };
        write_local(&mut out, &entry);
        out.extend_from_slice(&compressed);
        stored.push(entry);
    }
    zip_limit(
        out.len() as u64,
        "Central directory offset would require ZIP64 extensions",
    )?;
    let cd_off = out.len() as u32;
    for entry in &stored {
        write_central(&mut out, entry);
    }
    let cd_size = out.len() as u32 - cd_off;
    zip_limit(
        u64::from(cd_size),
        "Central directory size would require ZIP64 extensions",
    )?;
    write_eocd(&mut out, stored.len() as u16, cd_size, cd_off);
    Ok(out)
}

pub fn pack_apk(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, CompileError> {
    let unsigned = zip_entries(entries)?;
    sign_apk(&unsigned)
}

#[cfg(test)]
mod tests {
    use super::pack_apk;

    #[test]
    fn pack_two_tiny_entries() {
        let entries = [
            ("a.txt".to_string(), b"one".to_vec()),
            ("b.txt".to_string(), b"two".to_vec()),
        ];
        let apk = pack_apk(&entries).expect("pack apk");
        assert!(apk.starts_with(b"PK"));
        assert!(apk
            .windows(b"APK Sig Block 42".len())
            .any(|window| window == b"APK Sig Block 42"));
    }
}
