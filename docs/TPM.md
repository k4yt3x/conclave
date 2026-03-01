# TPM Support and Cipher Suite Configuration

## Prerequisites

Before implementing any changes described in this document, read **RFC 9420 (Messaging Layer Security)** in its entirety to understand the underlying protocol, particularly the sections on cipher suites (Section 5.1), key packages (Section 10), credentials (Section 5.3), and group operations (Sections 11-12).

**RFC 9420**: <https://www.rfc-editor.org/rfc/rfc9420.txt>

## 1. Background

Conclave uses MLS (RFC 9420) for end-to-end encryption. The current implementation hardcodes `CipherSuite::CURVE448_CHACHA` (MLS cipher suite 0x0007), which provides 256-bit security using X448, ChaCha20-Poly1305, SHA-512, and Ed448.

This cipher suite is incompatible with TPM 2.0 hardware. TPM 2.0 only supports NIST curves (P-256, P-384) for ECC operations, AES for symmetric encryption, and ECDSA for signatures. It does not support Curve448, Ed448, or ChaCha20-Poly1305. To enable TPM-backed key protection, the client must use a NIST curve cipher suite such as `P384_AES256`.

Two changes are needed:

1. **Configurable cipher suite** — allow administrators to choose the MLS cipher suite for their deployment, controlled by a config option on both the server and client.
2. **Optional TPM support** — store the MLS signing key in a TPM and derive a sealing key from the TPM to encrypt local databases at rest.

## 2. Cipher Suite Configuration

### 2.1 Available Cipher Suites

All 7 cipher suites defined by mls-rs are supported:

| Config Value | MLS ID | KEM | AEAD | Hash | Signature | Security |
|---|---|---|---|---|---|---|
| `CURVE25519_AES128` | 0x0001 | X25519 | AES-128-GCM | SHA-256 | Ed25519 | 128-bit |
| `P256_AES128` | 0x0002 | P-256 | AES-128-GCM | SHA-256 | ECDSA-P256 | 128-bit |
| `CURVE25519_CHACHA` | 0x0003 | X25519 | ChaCha20-Poly1305 | SHA-256 | Ed25519 | 128-bit |
| `CURVE448_AES256` | 0x0004 | X448 | AES-256-GCM | SHA-512 | Ed448 | 256-bit |
| `P384_AES256` | 0x0005 | P-384 | AES-256-GCM | SHA-384 | ECDSA-P384 | 192-bit |
| `P521_AES256` | 0x0006 | P-521 | AES-256-GCM | SHA-512 | ECDSA-P521 | 256-bit |
| `CURVE448_CHACHA` | 0x0007 | X448 | ChaCha20-Poly1305 | SHA-512 | Ed448 | 256-bit |

TPM-compatible suites: `P256_AES128`, `P384_AES256`, `P521_AES256` (P-521 has very limited real-world TPM support; see Section 3.8).

### 2.2 How MLS Cipher Suites Work

The cipher suite is baked into the MLS protocol at multiple levels:

- **Signing key**: The user's long-lived identity key uses the cipher suite's signature algorithm (e.g., Ed448 for `CURVE448_CHACHA`, ECDSA-P384 for `P384_AES256`). The signing key is generated at registration time and determines which cipher suite the user can participate in.
- **Key packages**: Each key package is tied to a specific cipher suite. When a user is invited to a group, the inviter fetches their key package. If the key package's cipher suite doesn't match the group's, the MLS library rejects the operation.
- **Groups**: Each group has a fixed cipher suite set at creation time. All members must use key packages with the matching cipher suite.

This means the cipher suite is effectively **per-deployment**: all users on a server must use the same cipher suite to communicate. Changing the cipher suite requires all users to perform an identity reset (`/reset`).

### 2.3 Server Enforcement

The server enforces a single cipher suite for the entire deployment. A `cipher_suite` config option (default: `"CURVE448_CHACHA"`) determines the allowed cipher suite. The server validates uploaded key packages by checking the cipher suite field in the MLS KeyPackage wire format (bytes 4-5, big-endian u16, immediately after the version and wire_format fields). Key packages with a mismatched cipher suite are rejected at upload time with a clear error message.

This prevents misconfigured clients from uploading unusable key packages that would only produce cryptic MLS errors when someone later tries to invite them.

```toml
# Server config
cipher_suite = "CURVE448_CHACHA"
```

### 2.4 Client Configuration

The client also has a `cipher_suite` config option (default: `"CURVE448_CHACHA"`) that must match the server's. The client uses this value when generating signing keys, creating key packages, and establishing groups.

```toml
# Client config
cipher_suite = "CURVE448_CHACHA"
```

If the client's cipher suite doesn't match the server's, the server rejects key package uploads at registration/login time, providing immediate feedback.

## 3. TPM Support

### 3.1 Overview

TPM (Trusted Platform Module) 2.0 support is an optional feature that provides hardware-backed key protection. When enabled:

1. The MLS signing key is stored as a **non-exportable key** inside the TPM. The TPM performs all signing operations; the private key never exists in software.
2. A **sealing key** is derived from the TPM and used to encrypt all local SQLite databases at rest (MLS state and message history).

### 3.2 What This Mitigates

| Threat | Without TPM | With TPM |
|---|---|---|
| Signing key theft from disk | Signing key file contains raw private key bytes; attacker copies and impersonates user from any machine | Key is non-exportable; file stores only a TPM key handle |
| Key package private key theft | ECDH private keys in MLS state database are readable | Database encrypted; unreadable without TPM |
| Epoch secret extraction from disk | MLS epoch secrets in state database are readable | Database encrypted; unreadable without TPM |
| Plaintext message history theft | Message history database contains decrypted message text | Database encrypted; unreadable without TPM |
| TOFU fingerprint tampering | TOFU fingerprint table can be modified by an attacker | Integrity-protected by authenticated encryption |
| Cold boot / stolen laptop (powered off) | All local data exposed | All sensitive data requires this specific TPM to decrypt |
| Disk moved to another machine | All data readable on any machine | Sealing key is bound to this TPM; data is unreadable elsewhere |

### 3.3 What This Does Not Mitigate

| Threat | Why TPM Doesn't Help |
|---|---|
| Live root attacker | Attacker waits for the application to unseal databases, then reads decrypted data from process memory |
| RAM forensics while app is running | Decrypted epoch secrets and messages are in memory during normal operation |
| Server-side attacks | Server trust model is orthogonal to client-side key storage |
| TPM hardware vulnerabilities | e.g., AMD fTPM voltage glitching attacks can extract sealing keys |
| Evil maid with sustained physical access | Can install a keylogger/implant, wait for user to boot and authenticate |

### 3.4 Dependencies

- **[tss-esapi](https://github.com/parallaxsecond/rust-tss-esapi)**: Rust wrapper for the TCG TSS 2.0 Enhanced System API. Provides ECC key creation, signing, sealing/unsealing, PCR policy, and persistent key management. Requires the `tpm2-tss` C library as a system dependency (`libtss2-dev` / `tpm2-tss-devel`).
- **[rusqlite](https://github.com/rusqlite/rusqlite)** with SQLCipher: Transparent AES-256 encryption for SQLite databases via the `bundled-sqlcipher` feature.

### 3.5 Compile-Time Feature Flag

TPM support is gated behind a `tpm` Cargo feature flag because `tss-esapi` introduces a C FFI dependency on `tpm2-tss`. This dependency should not be forced on builds that don't need TPM support. The feature is forwarded from the CLI and GUI binary crates to the client library crate.

```bash
cargo build --release --features tpm
```

### 3.6 Runtime Configuration

A `[tpm]` config section on the client controls TPM behavior at runtime:

```toml
[tpm]
enabled = false
pcr_policy = []
```

When `tpm.enabled = true`:

- The client validates at startup that the configured cipher suite is TPM-compatible (`P256_AES128`, `P384_AES256`, or `P521_AES256`). If not, exit with a clear error message.
- The signing key is created/loaded from the TPM instead of the filesystem.
- Databases are encrypted using a TPM-derived sealing key.

When compiled without the `tpm` feature, setting `tpm.enabled = true` in the config produces an error directing the user to rebuild with `--features tpm`.

### 3.7 Signing Key in TPM

#### Key Creation

On first registration (when no signing key exists):

1. Create a primary storage key under the TPM's owner hierarchy (`TPM2_CreatePrimary`).
2. Create a non-exportable ECC signing key as a child of the primary key (`TPM2_Create`) using the cipher suite's signature algorithm (e.g., `EccCurve::NistP384` + `EccScheme::EcDsa` for `P384_AES256`).
3. Load the key into the TPM (`TPM2_Load`).
4. Make the key persistent across reboots (`TPM2_EvictControl`) at a fixed handle.
5. Export the public key and construct a `SigningIdentity` with a `BasicCredential` (same as the current non-TPM flow).
6. Store the persistent key handle (not the private key) on disk.

#### Signing Operations

The mls-rs library's `CryptoProvider` trait provides a `CipherSuiteProvider` with a `sign()` method. TPM integration requires a wrapper provider that delegates `sign()` to the TPM via `TPM2_Sign` using the persistent key handle, while forwarding all other operations (ECDH, HKDF, AES) to the software crypto provider.

#### Identity Reset

On identity reset, delete the persistent TPM key (`TPM2_EvictControl` to remove from persistent storage), then create a new one following the creation flow above.

### 3.8 Database Encryption with TPM-Sealed Key

#### Sealing Key Lifecycle

On first startup with TPM enabled:

1. Generate 32 bytes of random data (the database encryption key).
2. Create a sealed data object in the TPM (`TPM2_Create` with sensitive data), optionally bound to PCR values if `pcr_policy` is configured.
3. Store the sealed blob on disk.

On each subsequent startup:

1. Load the sealed blob into the TPM (`TPM2_Load`).
2. If PCR policy is configured, create a policy session (`TPM2_StartAuthSession` + `TPM2_PolicyPCR`).
3. Unseal the encryption key (`TPM2_Unseal`).
4. Open SQLite databases with SQLCipher using the unsealed key via `PRAGMA key`.
5. Zeroize the unsealed key from memory after passing it to SQLCipher.

#### MLS State Database Complication

The MLS provider library (`mls-rs-provider-sqlite`) opens its own SQLite connection internally. Two options exist for encrypting this database:

- **Application-level file encryption**: Encrypt/decrypt the entire database file on open/close. The decrypted copy exists on disk during application runtime — simpler but provides weaker protection while the application is running.
- **Patch the MLS provider**: Fork or upstream a change to accept a SQLCipher-enabled connection. Cleaner but adds a maintenance burden.

### 3.9 TPM 2.0 Hardware Compatibility

Real-world TPM ECC curve support varies by implementation:

| TPM Implementation | ECC Curves Supported |
|---|---|
| Intel PTT (firmware TPM) | P-256, BN-P256 |
| AMD fTPM | P-256, P-384 (varies by generation) |
| STMicroelectronics discrete | P-256, P-384, BN-P256 |
| Nuvoton discrete | P-256, BN-P256 |

P-521 is defined in the TPM 2.0 specification but not implemented by any commonly deployed firmware or discrete TPM. **`P384_AES256` is the recommended cipher suite for TPM deployments** as it provides the strongest security level with broad TPM compatibility.

Users can query their TPM's supported curves with:

```bash
tpm2_getcap ecc-curves
```

### 3.10 PCR Policy (Optional)

When `pcr_policy` is configured with PCR indices (e.g., `[0, 7]`), the sealing key can only be unsealed when the specified PCR values match the state at sealing time. This binds database decryption to a specific boot configuration:

- **PCR 0**: UEFI firmware measurement
- **PCR 7**: Secure Boot state

If the boot chain changes (e.g., firmware update, Secure Boot disabled), the sealing key becomes inaccessible. The user would need to re-seal after verifying the new boot state is trustworthy.

## 4. Implementation Order

1. **Cipher suite configuration** (Section 2) — prerequisite for everything else. Make the cipher suite configurable on both server and client without any TPM dependency.
2. **TPM compile-time feature and runtime config** (Sections 3.5, 3.6) — add the compile-time and runtime plumbing.
3. **TPM signing key** (Section 3.7) — implement TPM-backed signing.
4. **Database encryption** (Section 3.8) — implement SQLCipher integration with TPM-sealed keys.
5. **PCR policy** (Section 3.10) — optional hardening, implement last.
