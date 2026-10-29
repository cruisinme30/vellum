---
id: 01M63A9ZS73ND4VBRAJW27K9NR
title: "ADR 0002: Envelope encryption format and key providers"
status: verified
owner: "@cruisinme30"
tags: [adr, security, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D34, D35, D36
- Issue: #47, #48, #49, #50
- Pull requests: #22, #29, #238

:::

## Context

The brief (section 11) makes envelope encryption at rest the default:
XChaCha20-Poly1305 data-encryption keys (DEKs) per document, wrapped by a
key-encryption key (KEK) from the OS keychain, a KMS or Vault, or an
Argon2id passphrase, with rotation by re-wrapping. This ADR fixes the
on-disk format and the provider contract in `vellum-crypto`.

## Decisions

### D34: one fresh DEK per stored object, header in the object

- Every object written to a blob store gets its own random 256-bit DEK and
  a random 192-bit nonce. A document's file and each of its versions are
  separate objects, so this is at least as fine-grained as "per document"
  and needs no DEK lookup table.
- The wrapped DEK and the KEK id travel in a header at the start of the
  object: magic `\0VLE`, version, KEK id, wrapped DEK, nonce, ciphertext.
  A Markdown file never starts with a NUL byte, so plaintext and sealed
  objects are told apart without metadata.

### D35: the payload is bound to neither the path nor the header

- The payload's associated data is a fixed label. Binding it to the path
  would break renames (file names are title slugs); binding it to the
  header would force re-encrypting every payload on rotation.
- Rotation (`rewrap`) unwraps the DEK with the old KEK, wraps it with the
  current one and rewrites only the header. Retired KEKs stay in the
  keyring until no header names them.

### D36: providers own the KEK and the wrapped form

- `KeyProvider` is async and treats the wrapped DEK as opaque bytes, so a
  KMS or Vault transit adapter wraps remotely and the KEK never leaves it.
- `LocalKeyring` serves the OS keychain and passphrase cases. A passphrase
  KEK id carries its Argon2id salt and costs, so the passphrase alone
  re-derives the KEK from any envelope header.

## Alternatives

- **One DEK per document in a lookup table**: an extra table to back up
  and keep in step, for no gain over one DEK per object.
- **Bind the payload to its path**: renames would need re-encryption.
- **Bind the payload to the header**: rotation would rewrite every
  payload instead of one header.

## Consequences

- Deleting one object's header shreds that object, and a backup
  restores keys and data together.
- Someone with write access to the store could swap two sealed objects;
  authorization, the audit log and content hashes cover that.
- `vellum-sealed-store` (engine layer) wraps any `BlobStore`: sealed
  writes by default, reads that accept both forms while a space switches
  mode, and `reseal` as the one migration and rotation step.
- `head`, `get`, `put` and `rename` report plaintext sizes; listings
  report stored sizes because they never read content.
- KEK backends: the OS keychain (macOS, Windows, Linux Secret Service), a
  passphrase from `VELLUM_KEK_PASSPHRASE` or `VELLUM_KEK_PASSPHRASE_FILE`,
  and in `vellum-kms` AWS KMS, Cloud KMS and Vault transit over HTTPS. Their
  KEK ids name the remote key and go along as associated data.
