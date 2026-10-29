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
- Consequence: deleting one object's header shreds that object, and a
  backup restores keys and data together.

### D35: the payload is bound to neither the path nor the header

- The payload's associated data is a fixed label. Binding it to the path
  would break renames (file names are title slugs); binding it to the
  header would force re-encrypting every payload on rotation.
- Rotation (`rewrap`) unwraps the DEK with the old KEK, wraps it with the
  current one and rewrites only the header. Retired KEKs stay in the
  keyring until no header names them.
- Consequence: someone with write access to the store could swap two whole
  sealed objects. That is an integrity threat to the store itself, covered
  by authorization, the audit log and the index's content hashes, not by
  encryption at rest.

### D36: providers own the KEK and the wrapped form

- `KeyProvider` is async and treats the wrapped DEK as opaque bytes, so a
  KMS or Vault transit adapter wraps remotely and the KEK never leaves it.
- `LocalKeyring` serves the OS keychain and passphrase cases. A passphrase
  KEK id carries its Argon2id salt and costs, so the passphrase alone
  re-derives the KEK from any envelope header.

## KEK backends

Issue [#48](https://github.com/cruisinme30/vellum/issues/48).

- Local KEKs: the OS keychain (macOS Keychain, Windows Credential
  Manager, the Linux Secret Service) or a passphrase read from
  `VELLUM_KEK_PASSPHRASE` or the file named by `VELLUM_KEK_PASSPHRASE_FILE`.
- Remote KEKs live in `vellum-kms`: AWS KMS, Google Cloud KMS and Vault
  transit, each a `KeyProvider` over plain HTTPS with no cloud SDK. The KEK
  id names the remote key, so old envelopes still open after a switch and
  `rewrap` moves them. The KEK id goes along as associated data, so a
  wrapped DEK opens only under the id its envelope names.
- Rotation inside the key service (new key versions) needs nothing from
  Vellum; moving to another key is a normal KEK rotation.

## Applying it to the blob store

`vellum-sealed-store` wraps any `BlobStore` in `Sealed`, an engine-layer
crate because `vellum-crypto` and `vellum-store` are both foundation.

- Writes follow the space's mode: sealed by default, as is in a plaintext
  space. Reads open an envelope wherever one is found and pass plaintext
  through, so a space stays readable while it switches mode. A plaintext
  object in an encrypted space is counted and logged until `reseal`
  rewrites it.
- `reseal` is the one migration and rotation step: it seals, re-wraps or
  opens an object as the policy now wants, guarded by `IfMatch`.
- Etags are the stored bytes' etags, so preconditions pass through. `get`,
  `head`, `put` and `rename` report plaintext sizes (`head` reads the
  object to find it); listings report stored sizes because they never
  read content.
