//! Minimal Widevine Content Decryption Module (L3) for SoundCloud's
//! `ctr-encrypted-hls` streams.
//!
//! Implements only what decryption needs: reading a `.wvd` device provision,
//! building and signing a licence request, unwrapping the licence's content
//! keys, and decrypting CENC samples. The protocol is Google's
//! `license_protocol.proto`; nothing here talks to a licence server, so the
//! HTTP part stays in `soundcloud/download.rs`.
//!
//! All of it is MIT and pure Rust: the `widevine`/`rswidevine` crates are
//! GPL-3.0 ports of pywidevine, which would relicense all of Wavekeep.

use aes::{Aes128, Aes192, Aes256};
use cbc::Decryptor;
use cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use cmac::Cmac;
use hmac::{Hmac, Mac};
use rand::RngCore;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pss::SigningKey;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::{Oaep, RsaPrivateKey};
use sha1::Sha1;
use sha2::Sha256;

/// `edef8ba9-79d6-4ace-a3c8-27dcd51d21ed`, the Widevine system ID.
const SYSTEM_ID: [u8; 16] = [
    0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d, 0x21, 0xed,
];
const LICENSE_STREAMING: u64 = 1;
const REQUEST_NEW: u64 = 1;
const MESSAGE_LICENSE_REQUEST: u64 = 1;
const MESSAGE_LICENSE: u64 = 2;
const MESSAGE_ERROR_RESPONSE: u64 = 3;
const PROTOCOL_VERSION_2_1: u64 = 21;
const KEY_TYPE_CONTENT: u64 = 2;

type Aes128CbcDec = Decryptor<Aes128>;

/// A provisioned Widevine device: its Google-signed client ID and private key.
/// Both are secrets in the sense that they identify a real device, so they are
/// never logged.
pub struct Device {
    client_id: Vec<u8>,
    key: RsaPrivateKey,
}

impl Device {
    /// Reads the `.wvd` container pywidevine and the Rust ports use: the magic
    /// `WVD`, a version (1 or 2), the device type, security level and flags,
    /// then the RSA key and the client ID, each length-prefixed (u16, BE).
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        const BAD: &str = "This file is not a Widevine device (.wvd)";
        if data.len() < 7 || &data[..3] != b"WVD" {
            return Err(BAD.to_string());
        }
        if data[3] != 1 && data[3] != 2 {
            return Err(BAD.to_string());
        }
        // Type, security level, flags.
        let (key_der, after_key) = length_prefixed(data, 7).ok_or(BAD)?;
        let (client_id, _) = length_prefixed(data, after_key).ok_or(BAD)?;
        if client_id.is_empty() {
            return Err(BAD.to_string());
        }
        let key = RsaPrivateKey::from_pkcs8_der(key_der)
            .or_else(|_| RsaPrivateKey::from_pkcs1_der(key_der))
            .map_err(|_| "The Widevine device file has an invalid private key".to_string())?;
        Ok(Self {
            client_id: client_id.to_vec(),
            key,
        })
    }
}

fn length_prefixed(data: &[u8], at: usize) -> Option<(&[u8], usize)> {
    let length = u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]) as usize;
    let start = at + 2;
    Some((data.get(start..start.checked_add(length)?)?, start + length))
}

/// The Widevine header from an HLS `#EXT-X-KEY` data URI: either a bare
/// `WidevineCencHeader` or a complete MP4 `pssh` box around one.
#[derive(Debug, Clone)]
pub struct Pssh {
    pub init_data: Vec<u8>,
    pub key_ids: Vec<[u8; 16]>,
}

pub fn parse_pssh(data: &[u8]) -> Result<Pssh, String> {
    let init_data = match pssh_box(data)? {
        Some(payload) => payload,
        None => data,
    };
    // WidevineCencHeader: repeated `key_id` is field 2.
    let key_ids = pb::fields(init_data)
        .into_iter()
        .filter(|field| field.number == 2)
        .filter_map(|field| field.bytes)
        .filter_map(|kid| <[u8; 16]>::try_from(kid).ok())
        .collect();
    Ok(Pssh {
        init_data: init_data.to_vec(),
        key_ids,
    })
}

/// The `init_data` of a version 0/1 `pssh` box, or `None` when `data` is not
/// a box at all.
fn pssh_box(data: &[u8]) -> Result<Option<&[u8]>, String> {
    if data.len() < 8 || &data[4..8] != b"pssh" {
        return Ok(None);
    }
    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let boxed = data.get(..size).unwrap_or(data);
    let version = *boxed.get(8).ok_or("SoundCloud sent a truncated PSSH box")?;
    let system_id = boxed
        .get(12..28)
        .ok_or("SoundCloud sent a truncated PSSH box")?;
    if system_id != SYSTEM_ID {
        return Err("SoundCloud sent a PSSH for a DRM system other than Widevine".to_string());
    }
    let mut at = 28;
    if version > 0 {
        let count = boxed
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or("SoundCloud sent a truncated PSSH box")?;
        at += 4 + count * 16;
    }
    let length = boxed
        .get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or("SoundCloud sent a truncated PSSH box")?;
    at += 4;
    boxed
        .get(at..at + length)
        .map(Some)
        .ok_or_else(|| "SoundCloud sent a truncated PSSH box".to_string())
}

/// One licence exchange: the signed challenge to POST, and the content key it
/// yields.
pub struct Challenge {
    device: Device,
    message: Vec<u8>,
    enc_context: Vec<u8>,
    mac_context: Vec<u8>,
    request_id: Vec<u8>,
    key_ids: Vec<[u8; 16]>,
}

impl Challenge {
    pub fn new(device: Device, pssh: &Pssh) -> Result<Self, String> {
        let mut request_id = vec![0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut request_id);

        // LicenseRequest.ContentIdentification.WidevinePsshData
        let mut pssh_data = Vec::new();
        pb::bytes(&mut pssh_data, 1, &pssh.init_data);
        pb::uint(&mut pssh_data, 2, LICENSE_STREAMING);
        pb::bytes(&mut pssh_data, 3, &request_id);
        let mut content_id = Vec::new();
        pb::bytes(&mut content_id, 1, &pssh_data);

        let mut request = Vec::new();
        pb::bytes(&mut request, 1, &device.client_id);
        pb::bytes(&mut request, 2, &content_id);
        pb::uint(&mut request, 3, REQUEST_NEW);
        pb::uint(&mut request, 4, unix_time());
        pb::uint(&mut request, 6, PROTOCOL_VERSION_2_1);
        // A POSITIVE int32; zero would look like an unset field.
        let nonce = u64::from(rand::random::<u32>() >> 1).max(1);
        pb::uint(&mut request, 7, nonce);

        let signature = sign(&device.key, &request)?;
        let mut message = Vec::new();
        pb::uint(&mut message, 1, MESSAGE_LICENSE_REQUEST);
        pb::bytes(&mut message, 2, &request);
        pb::bytes(&mut message, 3, &signature);

        Ok(Self {
            enc_context: context(b"ENCRYPTION", &request, 128),
            mac_context: context(b"AUTHENTICATION", &request, 512),
            request_id,
            key_ids: pssh.key_ids.clone(),
            device,
            message,
        })
    }

    /// The `SignedMessage` to send to the licence server.
    pub fn message(&self) -> &[u8] {
        &self.message
    }

    /// Verifies the licence and returns the 16-byte CENC content key that
    /// belongs to the PSSH (or the only content key it contains).
    pub fn content_key(&self, license: &[u8]) -> Result<[u8; 16], String> {
        let message = pb::fields(license);
        let kind = field_uint(&message, 1).unwrap_or(0);
        if kind == MESSAGE_ERROR_RESPONSE {
            return Err("SoundCloud refused the Widevine licence for this track".to_string());
        }
        if kind != MESSAGE_LICENSE {
            return Err("SoundCloud sent an unexpected Widevine licence".to_string());
        }
        let body = field_bytes(&message, 2).ok_or("SoundCloud sent an empty Widevine licence")?;
        let signature = field_bytes(&message, 3).ok_or("SoundCloud sent an unsigned Widevine licence")?;
        let wrapped = field_bytes(&message, 4).ok_or("SoundCloud sent a Widevine licence without a session key")?;
        let core = field_bytes(&message, 9).unwrap_or(&[]);

        let license = pb::fields(body);
        let answered = field_bytes(&license, 1)
            .and_then(|id| field_bytes(&pb::fields(id), 1))
            .unwrap_or(&[]);
        if answered != self.request_id.as_slice() {
            return Err("The Widevine licence does not answer this request".to_string());
        }

        let session_key = self
            .device
            .key
            .decrypt(Oaep::new::<Sha1>(), wrapped)
            .map_err(|_| "Could not unwrap the Widevine session key".to_string())?;
        let enc_key = derive(&session_key, &self.enc_context, 1)?;
        let mut mac_key = derive(&session_key, &self.mac_context, 1)?;
        mac_key.extend_from_slice(&derive(&session_key, &self.mac_context, 2)?);

        let mut mac = Hmac::<Sha256>::new_from_slice(&mac_key)
            .map_err(|_| "Invalid Widevine licence MAC key".to_string())?;
        mac.update(core);
        mac.update(body);
        mac.verify_slice(signature)
            .map_err(|_| "The Widevine licence signature does not match".to_string())?;

        let mut fallback: Option<[u8; 16]> = None;
        for container in license.iter().filter(|field| field.number == 3) {
            let Some(bytes) = container.bytes else {
                continue;
            };
            let fields = pb::fields(bytes);
            if field_uint(&fields, 4) != Some(KEY_TYPE_CONTENT) {
                continue;
            }
            let key = unlock_key(&enc_key, &fields)?;
            if self.key_ids.contains(&key.0) {
                return Ok(key.1);
            }
            fallback.get_or_insert(key.1);
        }
        fallback.ok_or_else(|| "The Widevine licence contains no content key".to_string())
    }
}

/// `(key id, key)` of one `License.KeyContainer`, decrypted with the licence's
/// encryption key.
fn unlock_key(enc_key: &[u8], fields: &[pb::Field<'_>]) -> Result<([u8; 16], [u8; 16]), String> {
    let id = field_bytes(fields, 1).unwrap_or(&[]);
    let iv = field_bytes(fields, 2).unwrap_or(&[]);
    let wrapped = field_bytes(fields, 3).ok_or("The Widevine licence key has no key material")?;

    let mut kid = [0u8; 16];
    let copied = id.len().min(16);
    kid[..copied].copy_from_slice(&id[..copied]);

    let mut block = [0u8; 16];
    let copied = iv.len().min(16);
    block[..copied].copy_from_slice(&iv[..copied]);

    let mut buffer = wrapped.to_vec();
    let plain = Aes128CbcDec::new_from_slices(enc_key, &block)
        .map_err(|_| "Invalid Widevine licence key".to_string())?
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|_| "Could not decrypt the Widevine content key".to_string())?;
    let key = <[u8; 16]>::try_from(plain).map_err(|_| "The Widevine content key is not 16 bytes".to_string())?;
    Ok((kid, key))
}

fn sign(key: &RsaPrivateKey, message: &[u8]) -> Result<Vec<u8>, String> {
    let signing = SigningKey::<Sha1>::new(key.clone());
    signing
        .try_sign_with_rng(&mut rand::rngs::OsRng, message)
        .map(|signature| signature.to_bytes().to_vec())
        .map_err(|e| format!("Could not sign the Widevine licence request: {}", e))
}

/// The AES-CMAC context pywidevine calls `_get_enc_context`/`_get_mac_context`.
fn context(label: &[u8], message: &[u8], key_bits: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(label.len() + message.len() + 5);
    out.extend_from_slice(label);
    out.push(0);
    out.extend_from_slice(message);
    out.extend_from_slice(&key_bits.to_be_bytes());
    out
}

fn derive(session_key: &[u8], context: &[u8], counter: u8) -> Result<Vec<u8>, String> {
    macro_rules! derive_with {
        ($cipher:ty) => {{
            let mut mac = <Cmac<$cipher> as Mac>::new_from_slice(session_key)
                .map_err(|_| "Invalid Widevine session key".to_string())?;
            mac.update(&[counter]);
            mac.update(context);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    match session_key.len() {
        16 => Ok(derive_with!(Aes128)),
        24 => Ok(derive_with!(Aes192)),
        32 => Ok(derive_with!(Aes256)),
        length => Err(format!("Unexpected Widevine session key length ({} bytes)", length)),
    }
}

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn field_uint(fields: &[pb::Field<'_>], number: u32) -> Option<u64> {
    fields
        .iter()
        .find(|field| field.number == number && field.bytes.is_none())
        .map(|field| field.value)
}

fn field_bytes<'a>(fields: &[pb::Field<'a>], number: u32) -> Option<&'a [u8]> {
    fields
        .iter()
        .find(|field| field.number == number)
        .and_then(|field| field.bytes)
}

/// CENC sample decryption for one stream, using a key from [`Challenge`].
pub struct Cenc {
    inner: vsd_mp4::decrypt::CencDecrypter,
}

impl Cenc {
    /// `init` is the HLS `#EXT-X-MAP` section, which carries the `tenc` box
    /// describing the encryption scheme.
    pub fn new(init: &[u8], key: &[u8; 16]) -> Result<Self, String> {
        let hex: String = key.iter().map(|byte| format!("{:02x}", byte)).collect();
        vsd_mp4::decrypt::CencDecrypter::with_init(&hex, init)
            .map(|inner| Self { inner })
            .map_err(|e| format!("Could not read the encrypted SoundCloud stream: {}", e))
    }

    /// Decrypts one fMP4 fragment (`moof` + `mdat`) in place.
    pub fn fragment(&self, fragment: Vec<u8>) -> Result<Vec<u8>, String> {
        self.inner
            .decrypt_fragment(fragment, None)
            .map_err(|e| format!("Could not decrypt the SoundCloud stream: {}", e))
    }
}

/// Just enough protobuf to read and write the licence messages by hand.
mod pb {
    #[derive(Debug, Clone, Copy)]
    pub struct Field<'a> {
        pub number: u32,
        pub bytes: Option<&'a [u8]>,
        pub value: u64,
    }

    pub fn bytes(out: &mut Vec<u8>, number: u32, data: &[u8]) {
        tag(out, number, 2);
        varint(out, data.len() as u64);
        out.extend_from_slice(data);
    }

    pub fn uint(out: &mut Vec<u8>, number: u32, value: u64) {
        tag(out, number, 0);
        varint(out, value);
    }

    fn tag(out: &mut Vec<u8>, number: u32, wire: u8) {
        varint(out, ((number as u64) << 3) | u64::from(wire));
    }

    fn varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn read_varint(mut data: &[u8]) -> Option<(u64, &[u8])> {
        let mut value = 0u64;
        for shift in 0..10 {
            let (first, rest) = data.split_first()?;
            value |= u64::from(first & 0x7f) << (shift * 7);
            data = rest;
            if first & 0x80 == 0 {
                return Some((value, data));
            }
        }
        None
    }

    /// Walks the fields of a serialized message. Truncated input ends the walk
    /// and yields what was read so far.
    pub fn fields(mut data: &[u8]) -> Vec<Field<'_>> {
        let mut out = Vec::new();
        while !data.is_empty() {
            let Some((header, rest)) = read_varint(data) else {
                break;
            };
            let number = (header >> 3) as u32;
            match header & 7 {
                0 => {
                    let Some((value, rest)) = read_varint(rest) else {
                        break;
                    };
                    out.push(Field { number, bytes: None, value });
                    data = rest;
                }
                1 => {
                    if rest.len() < 8 {
                        break;
                    }
                    out.push(Field { number, bytes: None, value: 0 });
                    data = &rest[8..];
                }
                2 => {
                    let Some((length, rest)) = read_varint(rest) else {
                        break;
                    };
                    let length = length as usize;
                    let Some(value) = rest.get(..length) else {
                        break;
                    };
                    out.push(Field { number, bytes: Some(value), value: 0 });
                    data = &rest[length..];
                }
                5 => {
                    if rest.len() < 4 {
                        break;
                    }
                    out.push(Field { number, bytes: None, value: 0 });
                    data = &rest[4..];
                }
                _ => break,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wvd(key: &RsaPrivateKey, client_id: &[u8]) -> Vec<u8> {
        use rsa::pkcs8::EncodePrivateKey;
        let der = key.to_pkcs8_der().unwrap();
        let mut out = b"WVD".to_vec();
        out.extend_from_slice(&[2, 2, 3, 0]); // version 2, Android, L3, no flags
        out.extend_from_slice(&(der.as_bytes().len() as u16).to_be_bytes());
        out.extend_from_slice(der.as_bytes());
        out.extend_from_slice(&(client_id.len() as u16).to_be_bytes());
        out.extend_from_slice(client_id);
        out
    }

    #[test]
    fn reads_a_device_provision() {
        use rsa::traits::PublicKeyParts;
        let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 1024).unwrap();
        let client_id = b"\x08\x01fake-client-identification".to_vec();
        let device = Device::parse(&wvd(&key, &client_id)).unwrap();
        assert_eq!(device.client_id, client_id);
        assert_eq!(device.key.n(), key.n());

        assert!(Device::parse(b"not a wvd").is_err());
        assert!(Device::parse(b"WVD\x09\x02\x03\x00").is_err());
    }

    #[test]
    fn reads_the_key_ids_of_a_bare_cenc_header() {
        let kid = [0x11u8; 16];
        let mut header = Vec::new();
        pb::uint(&mut header, 1, 1); // algorithm: AESCTR
        pb::bytes(&mut header, 2, &kid);
        pb::bytes(&mut header, 4, b"content-id");

        let pssh = parse_pssh(&header).unwrap();
        assert_eq!(pssh.key_ids, vec![kid]);
        assert_eq!(pssh.init_data, header);
    }

    #[test]
    fn unwraps_the_init_data_of_a_pssh_box() {
        let kid = [0x22u8; 16];
        let mut header = Vec::new();
        pb::bytes(&mut header, 2, &kid);
        let mut payload = Vec::new();
        payload.extend_from_slice(&(32 + header.len() as u32).to_be_bytes());
        payload.extend_from_slice(b"pssh");
        payload.extend_from_slice(&[0, 0, 0, 0]); // version 0, flags
        payload.extend_from_slice(&SYSTEM_ID);
        payload.extend_from_slice(&(header.len() as u32).to_be_bytes());
        payload.extend_from_slice(&header);

        let pssh = parse_pssh(&payload).unwrap();
        assert_eq!(pssh.key_ids, vec![kid]);
        assert_eq!(pssh.init_data, header);
    }

    #[test]
    fn refuses_another_drm_system() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&32u32.to_be_bytes());
        payload.extend_from_slice(b"pssh");
        payload.extend_from_slice(&[0, 0, 0, 0]);
        payload.extend_from_slice(&[0x9a; 16]); // PlayReady
        payload.extend_from_slice(&0u32.to_be_bytes());
        assert!(parse_pssh(&payload).is_err());
    }

    #[test]
    fn builds_a_signed_licence_challenge_in_protocol_order() {
        use rsa::signature::Verifier;
        let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 1024).unwrap();
        let device = Device::parse(&wvd(&key, b"\x08\x01client")).unwrap();
        let mut header = Vec::new();
        pb::bytes(&mut header, 2, &[0x33u8; 16]);
        let pssh = parse_pssh(&header).unwrap();

        let challenge = Challenge::new(device, &pssh).unwrap();
        let message = pb::fields(challenge.message());
        assert_eq!(field_uint(&message, 1), Some(MESSAGE_LICENSE_REQUEST));
        let request = field_bytes(&message, 2).unwrap();
        let signature = field_bytes(&message, 3).unwrap();

        let verifier = rsa::pss::VerifyingKey::<Sha1>::new(key.to_public_key());
        assert!(verifier.verify(request, &rsa::pss::Signature::try_from(signature).unwrap()).is_ok());

        let fields = pb::fields(request);
        assert_eq!(field_uint(&fields, 3), Some(REQUEST_NEW));
        assert_eq!(field_uint(&fields, 6), Some(PROTOCOL_VERSION_2_1));
        let content = field_bytes(&fields, 2).unwrap();
        let wv = field_bytes(&pb::fields(content), 1).unwrap();
        let wv = pb::fields(wv);
        assert_eq!(field_bytes(&wv, 1), Some(header.as_slice()));
        assert_eq!(field_uint(&wv, 2), Some(LICENSE_STREAMING));
        assert_eq!(field_bytes(&wv, 3).map(<[u8]>::len), Some(16));
    }

    #[test]
    fn derives_aes_cmac_keys_for_every_session_key_size() {
        for size in [16usize, 24, 32] {
            let key = vec![7u8; size];
            let other = vec![8u8; size];
            let context = b"AUTHENTICATION\0request\x00\x00\x02\x00";
            let first = derive(&key, context, 1).unwrap();
            assert_eq!(first.len(), 16);
            assert_ne!(first, derive(&key, context, 2).unwrap());
            assert_ne!(first, derive(&other, context, 1).unwrap());
        }
        assert!(derive(&[0u8; 20], b"x", 1).is_err());
    }

    #[test]
    fn parses_and_skips_unknown_protobuf_fields() {
        let mut message = Vec::new();
        pb::uint(&mut message, 1, 2);
        pb::bytes(&mut message, 2, b"licence");
        message.extend_from_slice(&[0x1d, 1, 2, 3, 4]); // field 3, fixed32
        message.extend_from_slice(&[0x21, 0, 0, 0, 0, 0, 0, 0, 0]); // field 4, fixed64
        let fields = pb::fields(&message);
        assert_eq!(field_uint(&fields, 1), Some(2));
        assert_eq!(field_bytes(&fields, 2), Some(b"licence".as_slice()));
        assert_eq!(pb::fields(&message[..3]).len(), 1);
    }
}
