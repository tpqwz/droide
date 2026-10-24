"""APK container and APK Signature Scheme v2 (min SDK 24, v2 only)."""

import hashlib
import io
import struct
import subprocess
import zipfile
from pathlib import Path

SIG_BLOCK_ID = 0x7109871A
SIG_MAGIC = b"APK Sig Block 42"
ALGO_RSA_PKCS1_SHA256 = 0x0103
CHUNK = 1024 * 1024
KEY_DIR = Path.home() / ".local" / "share" / "droide"


def _openssl(*args, input_bytes=None):
    proc = subprocess.run(
        ["openssl", *args],
        input=input_bytes,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.decode("utf-8", "replace") or "openssl failed")
    return proc.stdout


def debug_key():
    KEY_DIR.mkdir(parents=True, exist_ok=True)
    key = KEY_DIR / "debug.key.pem"
    cert = KEY_DIR / "debug.crt.pem"
    if not key.exists() or not cert.exists():
        _openssl(
            "req", "-newkey", "rsa:2048", "-keyout", str(key), "-x509",
            "-days", "3650", "-nodes", "-out", str(cert),
            "-subj", "/CN=Droide Debug",
        )
    pub = _openssl("pkey", "-in", str(key), "-pubout", "-outform", "DER")
    cer = _openssl("x509", "-in", str(cert), "-outform", "DER")
    return key, pub, cer


def _u32(value):
    return struct.pack("<I", value)


def _lp(blob):
    return _u32(len(blob)) + blob


def _seq(parts):
    """encodeAsSequenceOfLengthPrefixedElements: each part length-prefixed, no outer length."""
    return b"".join(_lp(part) for part in parts)


def _pairs(algo, value):
    """One length-prefixed (algorithm id, length-prefixed value) record. No outer list length."""
    return _lp(_u32(algo) + _lp(value))


def _chunk_count(size):
    if size <= 0:
        return 0
    return (size + CHUNK - 1) // CHUNK


def content_digest(sections):
    digests = []
    for section in sections:
        offset = 0
        remaining = len(section)
        count = _chunk_count(remaining)
        for _ in range(count):
            chunk = section[offset:offset + CHUNK]
            offset += len(chunk)
            digest = hashlib.sha256()
            digest.update(b"\xa5")
            digest.update(_u32(len(chunk)))
            digest.update(chunk)
            digests.append(digest.digest())
    top = hashlib.sha256()
    top.update(b"\x5a")
    top.update(_u32(len(digests)))
    for item in digests:
        top.update(item)
    return top.digest()


def _find_eocd(blob):
    # EOCD is the last 22 bytes unless there is a comment. Scan back for the signature.
    start = max(0, len(blob) - 22 - 65535)
    sig = b"PK\x05\x06"
    pos = blob.rfind(sig, start)
    if pos < 0:
        raise RuntimeError("ZIP end of central directory not found")
    return pos


def _signing_block(v2_value):
    pair = struct.pack("<QI", 4 + len(v2_value), SIG_BLOCK_ID) + v2_value
    size = len(pair) + 8 + 16
    return struct.pack("<Q", size) + pair + struct.pack("<Q", size) + SIG_MAGIC


def sign_apk(unsigned):
    eocd_off = _find_eocd(unsigned)
    eocd = bytearray(unsigned[eocd_off:])
    cd_size = struct.unpack_from("<I", eocd, 12)[0]
    cd_off = struct.unpack_from("<I", eocd, 16)[0]
    before = unsigned[:cd_off]
    central = unsigned[cd_off:cd_off + cd_size]
    key, public, certificate = debug_key()

    patched = bytearray(eocd)
    struct.pack_into("<I", patched, 16, len(before))
    apk_digest = content_digest([before, central, bytes(patched)])
    # Layout matches apksig V2SchemeSigner.generateSignerBlock.
    digests = _pairs(ALGO_RSA_PKCS1_SHA256, apk_digest)
    certs = _seq([certificate])
    signed_data = _seq([digests, certs, b"", b""])
    signature = _openssl("dgst", "-sha256", "-sign", str(key), input_bytes=signed_data)
    signatures = _pairs(ALGO_RSA_PKCS1_SHA256, signature)
    signer = _seq([signed_data, signatures, public])
    v2 = _seq([_seq([signer])])
    block = _signing_block(v2)
    struct.pack_into("<I", eocd, 16, len(before) + len(block))
    return before + block + central + bytes(eocd)


def pack_apk(entries):
    """entries: list of (name, bytes) in archive order. Returns signed APK bytes."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_DEFLATED, allowZip64=False) as zf:
        for name, data in entries:
            info = zipfile.ZipInfo(filename=name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            info.create_system = 0
            zf.writestr(info, data)
    return sign_apk(buf.getvalue())
