# Encrypted cloud backups

Cloud upload is an explicit desktop action. Select an existing local backup and enter a separate backup passphrase of at least 12 characters. The latest encrypted backup replaces the previous cloud backup for that signed-in account. Downloads create a new local backup; applying it requires the ordinary, explicit restore action.

Keep the backup passphrase somewhere safe. Account password recovery cannot recover this passphrase or decrypt an existing cloud backup. Use a strong, unique passphrase: someone with the encrypted envelope can try guesses offline. The app does not save the passphrase, derived key, or decrypted cloud payload to its settings or account credential. Passphrase and key buffers are cleared on drop on a best-effort basis; the UI must clear its password input after every attempt.

## Format v1

The desktop uses RustCrypto `argon2` and `chacha20poly1305`:

- Argon2id, version 0x13: 65,536 KiB memory, 3 iterations, parallelism 1, output 32 bytes.
- Fresh OS-random salt: 16 bytes per upload.
- XChaCha20Poly1305 with a fresh OS-random 24-byte nonce and a 16-byte authentication tag appended to ciphertext.
- Authenticated associated data, exactly UTF-8: `TrapLink cloud backup v1 | Argon2id v19 m=65536 t=3 p=1 | XChaCha20Poly1305`.
- Padded standard Base64 for `salt`, `nonce`, and `ciphertext`.
- Envelope: `{ "version": 1, "algorithm": "xchacha20poly1305", "salt": "...", "nonce": "...", "ciphertext": "..." }`.

KDF parameters are fixed by the format version and cannot be overridden by an untrusted server. Unknown versions are rejected. Decryption, authentication, and decoded document failures return the same error. The plaintext is the existing validated `BackupDocument`, which contains backup metadata, supported settings and profiles. It excludes account tokens, private keys and trusted peers.

Plaintext is capped at 5 MiB; Base64 and envelope overhead then fit the default 8 MiB server request limit. Serialization, response reads, decoded lengths and authentication are checked before applying data. Only one key derivation runs at a time per desktop process, and expensive cryptography runs on a blocking worker rather than the UI executor.

## API and integration

All cloud requests use the HTTPS origin tied to the signed-in device credential, and `Authorization: Bearer <deviceToken>`. HTTP is accepted only for localhost development. Redirects are disabled by the shared native client. Cloud operations take the account operation lock so logout, account changes and backup requests cannot race.

| Desktop command | Module function | Behavior |
| --- | --- | --- |
| `upload_cloud_backup({ id, passphrase })` | `save_cloud_backup` | Encrypt an existing local backup, PUT `/api/v1/backups/latest`, return `{updatedAt, bytes, sha256}`. |
| `download_cloud_backup({ passphrase })` | `fetch_cloud_backup` | GET `/api/v1/backups/latest`, authenticate and decrypt, return a validated document to the caller. The command saves it under a fresh local backup ID. |
| `delete_cloud_backup()` | `delete_cloud_backup` | DELETE `/api/v1/backups/latest` for the current account only. |

The server holds one opaque encrypted envelope per account and scopes reads, writes and deletion to an active, revocable device credential. The server can still observe account identity, encrypted size, timestamps and request activity. This format does not provide rollback protection against a server returning an older valid envelope. Local exported JSON backups remain ordinary unencrypted files and should be stored accordingly.

Crypto tests cover round trip, randomized encryption, wrong passphrase, ciphertext tampering, unsupported versions and bounded serialization.

References: [RustCrypto Argon2](https://docs.rs/argon2/0.5.3/argon2/) and [RustCrypto XChaCha20Poly1305](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/).
